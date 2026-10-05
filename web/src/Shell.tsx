import { useCallback, useEffect, useRef, useState } from "react";
import { api, errMsg } from "./api";
import { InstructorsModal } from "./admin/InstructorsModal";
import { MembersModal } from "./admin/MembersModal";
import { ScheduleScreen } from "./schedule/ScheduleScreen";
import { waitForJob } from "./schedule/solve";
import { useSession } from "./session";
import { useToast } from "./ui/Toast";
import type { Constraints, Row, ScheduleMeta, Season, Stats } from "./types";

const SEASON_KEY = "boatsched.season";
const remember = (id: number | null) => {
  try {
    if (id == null) localStorage.removeItem(SEASON_KEY);
    else localStorage.setItem(SEASON_KEY, String(id));
  } catch {
    // storage blocked: the season just isn't remembered
  }
};
const recall = (): number | null => {
  try {
    const v = localStorage.getItem(SEASON_KEY);
    return v ? Number(v) : null;
  } catch {
    return null;
  }
};

export function Shell() {
  const { me, cfg, logout } = useSession();
  const toast = useToast();
  const [seasons, setSeasons] = useState<Season[]>([]);
  const [seasonId, setSeasonId] = useState<number | null>(null);
  const [schedules, setSchedules] = useState<ScheduleMeta[]>([]);
  const [scheduleId, setScheduleId] = useState<number | null>(null);
  const [mode, setMode] = useState<"pick" | "new" | "view">("pick");
  const [stats, setStats] = useState<Stats | null>(null);
  const [staticRows, setStaticRows] = useState<Row[] | undefined>();
  const [solving, setSolving] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [error, setError] = useState("");
  const [modal, setModal] = useState<"members" | "pwd" | null>(null);
  const [newYear, setNewYear] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const timer = useRef<number | undefined>(undefined);

  const loadSchedules = useCallback(
    async (sid: number, open: "latest" | number | null) => {
      const list = await api<ScheduleMeta[]>(`/schedules?season_id=${sid}`);
      setSchedules(list);
      if (open === "latest" && list.length) {
        setScheduleId(list[0].id);
        setStats(null);
        setMode("view");
      } else if (typeof open === "number") {
        setScheduleId(open);
        setMode("view");
      }
    },
    [],
  );

  const loadSeasons = useCallback(async () => {
    const list = await api<Season[]>("/seasons");
    setSeasons(list);
    return list;
  }, []);

  const pickSeason = useCallback(
    (sid: number | null) => {
      setSeasonId(sid);
      remember(sid);
      setScheduleId(null);
      setStaticRows(undefined);
      setSchedules([]);
      setMode("pick");
      if (sid != null) loadSchedules(sid, "latest").catch((e) => setError(errMsg(e)));
    },
    [loadSchedules],
  );

  useEffect(() => {
    if (!cfg.db) return;
    loadSeasons()
      .then((list) => {
        const saved = recall();
        const pick = list.find((s) => s.id === saved)?.id ?? (list.length === 1 ? list[0].id : null);
        if (pick != null) pickSeason(pick);
      })
      .catch((e) => setError(errMsg(e)));
    // run once per login
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runJob = async (start: () => Promise<{ id: string }>) => {
    setError("");
    setSolving(true);
    setElapsed(0);
    const t0 = Date.now();
    timer.current = window.setInterval(() => setElapsed(Math.round((Date.now() - t0) / 1000)), 1000);
    try {
      const { id } = await start();
      const done = await waitForJob(id);
      setStats(done.stats ?? null);
      if (done.schedule_id != null && seasonId != null) {
        await loadSchedules(seasonId, done.schedule_id);
        setStaticRows(undefined);
      } else {
        setScheduleId(null);
        setStaticRows(done.result ?? []);
        setMode("view");
      }
    } catch (e) {
      setError(errMsg(e));
    } finally {
      window.clearInterval(timer.current);
      setSolving(false);
    }
  };

  const upload = (file: File) => {
    const form = new FormData();
    form.append("file", file);
    void runJob(() => api<{ id: string }>(`/solve${seasonId != null ? `?season_id=${seasonId}` : ""}`, { form }));
  };

  const resolve = (body: Constraints) => {
    if (scheduleId == null) return;
    void runJob(() => api<{ id: string }>(`/schedule/${scheduleId}/resolve`, { json: body }));
  };

  const createSeason = async () => {
    const year = parseInt(newYear ?? "", 10);
    if (!year) return;
    try {
      const s = await api<Season>("/seasons", { json: { year } });
      setNewYear(null);
      await loadSeasons();
      pickSeason(s.id);
    } catch (e) {
      toast(errMsg(e), "error");
    }
  };

  const canUpload = me.is_admin && (!cfg.db || seasonId != null);

  return (
    <div className="wrap">
      <header className="top">
        {cfg.db && (
          <div className="user-bar">
            <button className="btn-sm" onClick={() => setModal("pwd")}>{me.is_admin ? "Wachtwoorden" : "Mijn wachtwoord"}</button>
            {me.is_admin && <button className="btn-sm" onClick={() => setModal("members")}>Leden</button>}
            <button className="btn-sm" onClick={() => void logout()}>Uitloggen</button>
          </div>
        )}
        <div className="kicker">Vloot Rooster · Dagindeling</div>
        <h1>Boot Rooster<span className="sub">— hijs de zeilen —</span></h1>
      </header>
      <div className="rule" />

      {solving ? (
        <div className="spin" role="status">
          <div className="ring" />
          <div className="big">Vloot wordt ingedeeld…</div>
          <p>Gemengde boten minimaliseren, diversiteit maximaliseren, herhalingen verminderen.</p>
          <div className="statline">verstreken {elapsed}s</div>
        </div>
      ) : (
        <>
          {cfg.db && (
            <div className="season-bar">
              <div className="season-row">
                <label htmlFor="season-pick">Seizoen</label>
                <select id="season-pick" className="sel" value={seasonId ?? ""} onChange={(e) => pickSeason(e.target.value ? Number(e.target.value) : null)}>
                  <option value="">— kies een seizoen —</option>
                  {seasons.map((s) => <option key={s.id} value={s.id}>{s.year}</option>)}
                </select>
                {me.is_admin && <button className="btn-sm" onClick={() => setNewYear("")}>+ Nieuw</button>}
              </div>
              {newYear !== null && (
                <form className="season-row" onSubmit={(e) => { e.preventDefault(); void createSeason(); }}>
                  <input className="yr" type="number" placeholder="Jaar" min={2000} max={2099} autoFocus aria-label="Jaar" value={newYear} onChange={(e) => setNewYear(e.target.value)} />
                  <button type="submit" className="btn-sm primary">Aanmaken</button>
                  <button type="button" className="btn-sm" onClick={() => setNewYear(null)}>Annuleren</button>
                </form>
              )}
              {seasonId == null && <div className="season-hint">Selecteer of maak een seizoen aan.</div>}
              {seasonId != null && (
                <div className="season-row">
                  <label htmlFor="schedule-pick">Rooster</label>
                  <select
                    id="schedule-pick"
                    className="sel"
                    value={mode === "new" ? "new" : scheduleId ?? ""}
                    onChange={(e) => {
                      const v = e.target.value;
                      setError("");
                      if (v === "new") { setMode("new"); setScheduleId(null); setStaticRows(undefined); }
                      else if (v) { setScheduleId(Number(v)); setStats(null); setMode("view"); }
                      else { setMode("pick"); setScheduleId(null); }
                    }}
                  >
                    <option value="">— selecteer —</option>
                    {me.is_admin && <option value="new">+ Nieuw rooster uploaden</option>}
                    {schedules.map((s) => <option key={s.id} value={s.id}>{(s.created ?? "").slice(0, 16)}</option>)}
                  </select>
                </div>
              )}
              {seasonId != null && schedules.length === 0 && mode === "pick" && (
                <div className="season-hint">{me.is_admin ? "Nog geen roosters: kies “Nieuw rooster uploaden”." : "Er zijn nog geen roosters voor dit seizoen."}</div>
              )}
            </div>
          )}

          {error && <p className="err" role="alert">⚠ {error}</p>}

          {canUpload && (mode === "new" || (!cfg.db && !staticRows)) && (
            <label
              className={`drop${over ? " over" : ""}`}
              onDragOver={(e) => { e.preventDefault(); setOver(true); }}
              onDragLeave={() => setOver(false)}
              onDrop={(e) => { e.preventDefault(); setOver(false); const f = e.dataTransfer.files[0]; if (f) upload(f); }}
            >
              <span className="anchor">⚓</span>
              <h2>Sleep uw rooster CSV hier</h2>
              <p>of klik om te kiezen · kolommen: naam, niveau · de server lost het op</p>
              <input type="file" accept=".csv,text/csv" onChange={(e) => { const f = e.target.files?.[0]; if (f) upload(f); }} />
            </label>
          )}

          {mode === "view" && (scheduleId != null || staticRows) && (
            <>
              {me.is_admin && (
                <div className="controls">
                  <button className="btn-sm" onClick={() => { setMode("new"); setScheduleId(null); setStaticRows(undefined); }}>
                    ↻ Nieuw rooster uploaden
                  </button>
                </div>
              )}
              <ScheduleScreen scheduleId={scheduleId} seasonId={seasonId} stats={stats} staticRows={staticRows} onResolve={resolve} />
            </>
          )}
        </>
      )}

      {modal === "members" && <MembersModal seasonId={seasonId} onClose={() => setModal(null)} />}
      {modal === "pwd" && <InstructorsModal onClose={() => setModal(null)} />}
    </div>
  );
}
