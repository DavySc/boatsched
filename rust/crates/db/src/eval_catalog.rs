//! The skill catalog — the single source of truth for the evaluation cards.
//!
//! Transcribed from the four `getSkillsForLevel` matrices that used to be
//! hardcoded in `viewer.html`. The catalog is seeded into the `eval_skill_*`
//! tables once (see [`seed_catalog`]); thereafter the DB is authoritative and
//! the frontend fetches it via `GET /api/skills`.
//!
//! Categories carry an `eval_type` (`"standard"` cells cycle `•`/`✓`,
//! `"overall"` cells cycle `O`/`T`/`V`/`G`); skills may have sub-skills.

use rusqlite::{params, Connection};

/// A skill (or sub-skill, via [`CatalogSkill::subskills`]) within a category.
pub struct CatalogSkill {
    pub label: &'static str,
    pub subskills: &'static [&'static str],
}

/// A category groups skills and fixes how their cells are scored.
pub struct CatalogCategory {
    pub name: &'static str,
    /// `"standard"` or `"overall"`.
    pub eval_type: &'static str,
    pub skills: &'static [CatalogSkill],
}

/// One enrollment level's worth of categories.
pub struct CatalogLevel {
    /// `"k" | "b" | "A" | "B"`.
    pub level: &'static str,
    pub categories: &'static [CatalogCategory],
}

/// Statuses a cell may hold for a given category `eval_type`. Used for both the
/// `CHECK` semantics and server-side validation. Absence of a row = "not set".
pub fn allowed_statuses(eval_type: &str) -> &'static [&'static str] {
    match eval_type {
        "overall" => &["O", "T", "V", "G"],
        _ => &["attempted", "completed"],
    }
}

macro_rules! skill {
    ($label:literal) => {
        CatalogSkill { label: $label, subskills: &[] }
    };
    ($label:literal, [$($sub:literal),* $(,)?]) => {
        CatalogSkill { label: $label, subskills: &[$($sub),*] }
    };
}

/// The full catalog for all four levels.
pub static CATALOG: &[CatalogLevel] = &[
    // ---------------------------------------------------------------- k
    CatalogLevel {
        level: "k",
        categories: &[
            CatalogCategory {
                name: "Schip in rust",
                eval_type: "standard",
                skills: &[
                    skill!("Zeilklaar maken grootzeil", ["Vallen aanslaan", "Huikje"]),
                    skill!("Zeilklaar maken fok", ["Fok hijsen"]),
                    skill!("Hijsen grootzeil", ["Volgorde", "Hijsen", "Afwerking"]),
                    skill!("Hijsen fok"),
                    skill!("Strijken grootzeil", ["Volgorde", "Snelheid", "Zeil opdoeken"]),
                    skill!("Strijken fok"),
                    skill!("Afmeren Box Rufus", ["Volgorde", "Afwerking"]),
                    skill!("Nachtklaar maken", ["Huikje", "Schoten/ vallen", "Breien", "Poetsen"]),
                    skill!("Verhalen schip langs de wal"),
                    skill!("Verhalen schip 180º"),
                    skill!("Bomen"),
                    skill!("Knopen"),
                    skill!("Peddelen"),
                ],
            },
            CatalogCategory {
                name: "Basisvaardigheden",
                eval_type: "standard",
                skills: &[
                    skill!("Sturen"),
                    skill!("Koersen", ["Windrichting", "Namen", "Oploeven/ afvallen", "Grenskoersen VDW", "Grenskoersen ADW"]),
                    skill!("Zeilstanden", ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]),
                    skill!("Overstag", ["ADW", "Commando's", "Sturen"]),
                    skill!("Dwarspeiling"),
                    skill!("Gijpen", ["VDW", "Commando's", "Werking roer"]),
                ],
            },
            CatalogCategory {
                name: "Bijzondere verrichtingen",
                eval_type: "standard",
                skills: &[skill!("Eilandje ronden"), skill!("Achtje varen")],
            },
            CatalogCategory {
                name: "Algemene indruk",
                eval_type: "overall",
                skills: &[skill!("Algemene indruk", ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"])],
            },
        ],
    },
    // ---------------------------------------------------------------- b
    CatalogLevel {
        level: "b",
        categories: &[
            CatalogCategory {
                name: "Schip in rust",
                eval_type: "standard",
                skills: &[
                    skill!("Zeilklaar maken grootzeil", ["Vallen aanslaan", "Huikje"]),
                    skill!("Zeilklaar maken fok", ["Fok hijsen"]),
                    skill!("Hijsen grootzeil", ["Volgorde", "Hijsen", "Afwerking"]),
                    skill!("Hijsen fok"),
                    skill!("Strijken grootzeil", ["Volgorde", "Snelheid", "Zeil opdoeken"]),
                    skill!("Strijken fok"),
                    skill!("Afmeren Box Rufus", ["Volgorde", "Afwerking"]),
                    skill!("Nachtklaar maken", ["Huikje/ schoten/ vallen", "Breien/ poetsen"]),
                    skill!("Verhalen schip langs de wal"),
                    skill!("Verhalen schip 180º"),
                    skill!("Bomen"),
                    skill!("Knopen"),
                    skill!("Peddelen"),
                ],
            },
            CatalogCategory {
                name: "Basisvaardigheden",
                eval_type: "standard",
                skills: &[
                    skill!("Sturen"),
                    skill!("Koersen", ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]),
                    skill!("Zeilstanden", ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]),
                    skill!("Overstag", ["ADW", "Commando's", "Sturen"]),
                    skill!("Dwarspeiling"),
                    skill!("Gijpen", ["VDW", "Commando's", "Werking roer"]),
                    skill!("BPR"),
                ],
            },
            CatalogCategory {
                name: "Bijzondere verrichtingen",
                eval_type: "standard",
                skills: &[
                    skill!("Aanleggen hogerwal", ["ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]),
                    skill!("Afvaren hogerwal", ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]),
                    skill!("MOB", ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]),
                    skill!("Stormronde/ halzen"),
                ],
            },
            CatalogCategory {
                name: "Algemene indruk",
                eval_type: "overall",
                skills: &[skill!("Algemene indruk", ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"])],
            },
        ],
    },
    // ---------------------------------------------------------------- A
    CatalogLevel {
        level: "A",
        categories: &[
            CatalogCategory {
                name: "Schip in rust",
                eval_type: "standard",
                skills: &[
                    skill!("Zeilklaar maken grootzeil"),
                    skill!("Hijsen grootzeil"),
                    skill!("Hijsen fok"),
                    skill!("Strijken grootzeil"),
                    skill!("Strijken fok"),
                    skill!("Afmeren"),
                    skill!("Nachtklaar maken"),
                    skill!("Verhalen schip"),
                    skill!("Bomen"),
                    skill!("Knopen"),
                    skill!("Reven"),
                ],
            },
            CatalogCategory {
                name: "Basisvaardigheden",
                eval_type: "standard",
                skills: &[
                    skill!("Sturen"),
                    skill!("Koersen", ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]),
                    skill!("Zeilstanden", ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]),
                    skill!("Overstag"),
                    skill!("Dwarspeiling"),
                    skill!("Gijpen"),
                    skill!("BPR"),
                    skill!("Knopen"),
                ],
            },
            CatalogCategory {
                name: "Bijzondere verrichtingen",
                eval_type: "standard",
                skills: &[
                    skill!("Aanleggen hogerwal", ["Dwarspeiling meth.", "Opdraai meth.", "Opschieter meth.", "ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]),
                    skill!("Afvaren hogerwal", ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]),
                    skill!("MOB", ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]),
                    skill!("Varend hijsen/ strijken", ["Organisatie", "Koers", "Kraanlijn"]),
                    skill!("Kruisen nauw water", ["Korte slag", "Lange slag", "BPR", "Hoogtelopen"]),
                    skill!("Mast strijken"),
                    skill!("Mast zetten"),
                ],
            },
            CatalogCategory {
                name: "Algemene indruk",
                eval_type: "overall",
                skills: &[skill!("Algemene indruk", ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"])],
            },
        ],
    },
    // ---------------------------------------------------------------- B
    CatalogLevel {
        level: "B",
        categories: &[
            CatalogCategory {
                name: "Schip in rust",
                eval_type: "standard",
                skills: &[
                    skill!("Zeilklaar maken grootzeil"),
                    skill!("Hijsen grootzeil"),
                    skill!("Hijsen fok"),
                    skill!("Strijken grootzeil"),
                    skill!("Strijken fok"),
                    skill!("Afmeren"),
                    skill!("Nachtklaar maken"),
                    skill!("Verhalen schip"),
                    skill!("Bomen"),
                    skill!("Knopen"),
                    skill!("Reven"),
                ],
            },
            CatalogCategory {
                name: "Basisvaardigheden",
                eval_type: "standard",
                skills: &[
                    skill!("Sturen"),
                    skill!("Koersen", ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]),
                    skill!("Zeilstanden", ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]),
                    skill!("Overstag"),
                    skill!("Dwarspeiling"),
                    skill!("Gijpen"),
                    skill!("BPR"),
                    skill!("Knopen"),
                ],
            },
            CatalogCategory {
                name: "Bijzondere verrichtingen",
                eval_type: "standard",
                skills: &[
                    skill!("Aanleggen hogerwal", ["Dwarspeiling meth.", "Opdraai meth.", "Opschieter meth.", "ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]),
                    skill!("Afvaren hogerwal", ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]),
                    skill!("MOB", ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]),
                    skill!("Kruisen nauw water", ["Korte slag", "Lange slag", "BPR", "Hoogtelopen"]),
                    skill!("Varend reven"),
                    skill!("Loskomen"),
                    skill!("Aanleggen lagerwal", ["Parcours", "Organisatie", "Inzicht", "Strijken"]),
                    skill!("Afvaren lagerwal", ["Parcours", "Organisatie", "Inzicht", "Afduwen", "Hijsen"]),
                    skill!("Ankeren", ["Parcours", "Organisatie", "Deinzen", "Ankermaat"]),
                    skill!("Ankerop gaan", ["Organisatie", "Inzicht", "Ankermaat"]),
                ],
            },
            CatalogCategory {
                name: "Algemene indruk",
                eval_type: "overall",
                skills: &[skill!("Algemene indruk", ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"])],
            },
        ],
    },
];

/// Seed the catalog into `eval_skill_category` / `eval_skill` — but only if it
/// is empty (idempotent). Runs in a single transaction. Ordinals follow the
/// declaration order above so the UI renders in the intended sequence.
pub fn seed_catalog(conn: &Connection) -> rusqlite::Result<()> {
    let already: i64 =
        conn.query_row("SELECT COUNT(*) FROM eval_skill_category", [], |r| r.get(0))?;
    if already > 0 {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for level in CATALOG {
        for (cat_ord, cat) in level.categories.iter().enumerate() {
            tx.execute(
                "INSERT INTO eval_skill_category (level, name, eval_type, ordinal) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![level.level, cat.name, cat.eval_type, cat_ord as i64],
            )?;
            let category_id = tx.last_insert_rowid();
            for (skill_ord, sk) in cat.skills.iter().enumerate() {
                tx.execute(
                    "INSERT INTO eval_skill (category_id, parent_id, label, ordinal) \
                     VALUES (?1, NULL, ?2, ?3)",
                    params![category_id, sk.label, skill_ord as i64],
                )?;
                let skill_id = tx.last_insert_rowid();
                for (sub_ord, sub) in sk.subskills.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO eval_skill (category_id, parent_id, label, ordinal) \
                         VALUES (?1, ?2, ?3, ?4)",
                        params![category_id, skill_id, sub, sub_ord as i64],
                    )?;
                }
            }
        }
    }
    tx.commit()?;
    Ok(())
}
