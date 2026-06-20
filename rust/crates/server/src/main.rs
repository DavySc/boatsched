//! `boatsched-server` binary — a port of `server.py:main`.
//!
//! Parses CLI args, opens/creates the DB, applies production hardening when a
//! `--db` is configured (overridable via env), seeds the bootstrap admin from
//! `BOATSCHED_ADMIN`/`BOATSCHED_ADMIN_PW`, and serves the axum app.

use boatsched_core::{svec, Config};
use boatsched_server::{create_app, ServerOptions};

fn env_bool(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(
            v.trim().to_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        Err(_) => default,
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

struct Args {
    port: u16,
    host: String,
    cfg: Config,
    db: Option<String>,
}

fn parse_args() -> Args {
    // Minimal flag parser mirroring server.py:parse_args defaults. Multi-value
    // flags (--levels/--restricted/--diverse) take the rest of the args until
    // the next flag.
    let mut port = 8000u16;
    let mut host = "0.0.0.0".to_string();
    let mut cfg = Config {
        stage_seconds: 45.0,
        seconds: 60.0,
        ..Config::default()
    };
    let mut db = None;
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let take_list = |argv: &[String], i: &mut usize| -> Vec<String> {
        let mut out = Vec::new();
        while *i + 1 < argv.len() && !argv[*i + 1].starts_with("--") {
            *i += 1;
            out.push(argv[*i].clone());
        }
        out
    };
    while i < argv.len() {
        match argv[i].as_str() {
            "--port" => {
                i += 1;
                port = argv[i].parse().unwrap_or(8000);
            }
            "--host" => {
                i += 1;
                host = argv[i].clone();
            }
            "--boats" => {
                i += 1;
                cfg.boats = argv[i].parse().unwrap_or(13);
            }
            "--adj" => {
                i += 1;
                cfg.adj = argv[i].parse().unwrap_or(1);
            }
            "--stage-seconds" => {
                i += 1;
                cfg.stage_seconds = argv[i].parse().unwrap_or(45.0);
            }
            "--seconds" => {
                i += 1;
                cfg.seconds = argv[i].parse().unwrap_or(60.0);
            }
            "--workers" => {
                i += 1;
                cfg.workers = argv[i].parse().unwrap_or(8);
            }
            "--db" => {
                i += 1;
                db = Some(argv[i].clone());
            }
            "--levels" => {
                cfg.levels = take_list(&argv, &mut i);
            }
            "--restricted" => {
                cfg.restricted = take_list(&argv, &mut i);
            }
            "--diverse" => {
                cfg.diverse = take_list(&argv, &mut i);
            }
            other => eprintln!("[warn] ignoring unknown arg: {other}"),
        }
        i += 1;
    }
    if cfg.levels.is_empty() {
        cfg.levels = svec(&["k", "b", "A", "B"]);
    }
    Args {
        port,
        host,
        cfg,
        db,
    }
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = parse_args();

    let db_conn = match &args.db {
        Some(path) => match boatsched_db::init_db(path) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("[error] could not open database {path}: {e}");
                return std::process::ExitCode::FAILURE;
            }
        },
        None => None,
    };

    let production = db_conn.is_some();
    let require_auth = env_bool("BOATSCHED_REQUIRE_AUTH", production);
    let secure_cookies = env_bool("BOATSCHED_SECURE_COOKIES", production);
    let min_password_len = env_usize("BOATSCHED_MIN_PASSWORD_LEN", if production { 8 } else { 1 });
    let login_max_failures = env_usize("BOATSCHED_LOGIN_MAX_FAIL", 10);

    let opts = ServerOptions {
        require_auth,
        secure_cookies,
        min_password_len,
        login_max_failures,
        ..ServerOptions::default()
    };

    // Bootstrap admin BEFORE moving the connection into the app (create_app
    // runs migrations first; do it here in the equivalent order).
    if let Some(conn) = &db_conn {
        boatsched_db::schema::run_migrations(conn).expect("migrations");
        match (
            std::env::var("BOATSCHED_ADMIN"),
            std::env::var("BOATSCHED_ADMIN_PW"),
        ) {
            (Ok(name), Ok(pw)) if !name.is_empty() && !pw.is_empty() => {
                match boatsched_db::ensure_admin(conn, &name, &pw) {
                    Ok(seeded) => println!(
                        "  bootstrap admin '{name}' {}",
                        if seeded { "seeded" } else { "present" }
                    ),
                    Err(e) => {
                        eprintln!("[error] admin bootstrap failed: {e:?}");
                        return std::process::ExitCode::FAILURE;
                    }
                }
            }
            _ if require_auth => {
                let has_login: Option<i64> = conn
                    .query_row(
                        "SELECT 1 FROM instructor WHERE password_hash IS NOT NULL LIMIT 1",
                        [],
                        |r| r.get(0),
                    )
                    .ok();
                if has_login.is_none() {
                    eprintln!(
                        "[warn] require_auth is on but no admin exists and \
                         BOATSCHED_ADMIN/BOATSCHED_ADMIN_PW are unset — nobody \
                         can log in. Set them and restart."
                    );
                }
            }
            _ => {}
        }
    }

    let cfg = args.cfg.clone();
    let app = create_app(args.cfg, db_conn, opts);

    println!("boatsched server at http://localhost:{}", args.port);
    println!("  boats={} levels={:?}", cfg.boats, cfg.levels);
    println!(
        "  restricted={:?} diverse={:?}",
        cfg.restricted, cfg.diverse
    );
    println!(
        "  require_auth={require_auth} secure_cookies={secure_cookies} \
         min_password_len={min_password_len}"
    );
    println!("Ctrl-C to stop.");

    let addr = format!("{}:{}", args.host, args.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[error] could not bind {addr}: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[error] server error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
