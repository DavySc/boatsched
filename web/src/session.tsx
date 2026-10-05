import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { ApiError, api, setUnauthorizedHandler } from "./api";
import type { AppConfig, Boat, Me } from "./types";

interface Session {
  me: Me;
  cfg: AppConfig;
  boats: Record<number, string | null>;
  renameBoat: (id: number, name: string | null) => Promise<void>;
  logout: () => Promise<void>;
}

const Ctx = createContext<Session | null>(null);

export function useSession(): Session {
  const s = useContext(Ctx);
  if (!s) throw new Error("useSession outside <SessionProvider>");
  return s;
}

const DEFAULT_CFG: AppConfig = {
  boats: 13,
  levels: ["k", "b", "A", "B"],
  restricted: ["Sat", "Sun", "Tue", "Wed"],
  diverse: ["Fri", "Mon"],
  db: false,
};

/** Resolves the current session; renders `login` when there is none. */
export function SessionProvider({
  login,
  children,
}: {
  login: (onDone: () => void) => ReactNode;
  children: ReactNode;
}) {
  const [state, setState] = useState<"loading" | "anon" | "ready">("loading");
  const [me, setMe] = useState<Me | null>(null);
  const [cfg, setCfg] = useState<AppConfig>(DEFAULT_CFG);
  const [boats, setBoats] = useState<Record<number, string | null>>({});

  const load = useCallback(async () => {
    try {
      const who = await api<Me>("/api/auth/me");
      const [config, boatList] = await Promise.all([
        api<AppConfig>("/config"),
        api<Boat[]>("/boats").catch(() => [] as Boat[]),
      ]);
      setMe(who);
      setCfg(config);
      setBoats(Object.fromEntries(boatList.map((b) => [b.id, b.name || null])));
      setState("ready");
    } catch (e) {
      if (e instanceof ApiError && e.status === 401) setState("anon");
      else throw e;
    }
  }, []);

  useEffect(() => {
    setUnauthorizedHandler(() => setState("anon"));
    load().catch(() => setState("anon"));
    return () => setUnauthorizedHandler(null);
  }, [load]);

  const value = useMemo<Session | null>(() => {
    if (!me) return null;
    return {
      me,
      cfg,
      boats,
      renameBoat: async (id, name) => {
        await api("/boat", { json: { id, name } });
        setBoats((b) => ({ ...b, [id]: name }));
      },
      logout: async () => {
        await api("/api/auth/logout", { method: "POST" }).catch(() => {});
        setMe(null);
        setState("anon");
      },
    };
  }, [me, cfg, boats]);

  if (state === "loading") return <p className="center-note">Laden…</p>;
  if (state === "anon" || !value) return <>{login(() => void load())}</>;
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
