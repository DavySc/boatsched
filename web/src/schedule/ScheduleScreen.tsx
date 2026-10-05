import { useEffect, useMemo, useState } from "react";
import { api, errMsg } from "../api";
import { useSession } from "../session";
import { sortDays } from "../i18n";
import { EvalCard } from "../eval/EvalCard";
import { useToast } from "../ui/Toast";
import type { CommitResult, Constraints, Row, ScheduleData, Stats } from "../types";
import { AbsenceEditor } from "./AbsenceEditor";
import { BoatView, type BoatActions } from "./BoatView";
import { ConflictPanel } from "./ConflictPanel";
import { KidView } from "./KidView";
import { cellKey, computeConflicts } from "./conflicts";
import { useYSchedule } from "./useYSchedule";

interface Props {
  scheduleId: number | null;
  seasonId: number | null;
  stats: Stats | null;
  /** Rows of a solve that was not saved (no database): shown read-only. */
  staticRows?: Row[];
  onResolve: (body: Constraints) => void;
}

/** Loads a saved schedule, then hands over to the live (collaborative) view. */
export function ScheduleScreen(p: Props) {
  const [data, setData] = useState<ScheduleData | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setData(null);
    setError(null);
    if (p.scheduleId == null) return;
    let cancelled = false;
    api<ScheduleData>(`/schedule?id=${p.scheduleId}`)
      .then((d) => !cancelled && setData(d))
      .catch((e) => !cancelled && setError(errMsg(e)));
    return () => {
      cancelled = true;
    };
  }, [p.scheduleId]);

  if (p.scheduleId == null)
    return (
      <Body
        rows={p.staticRows ?? []}
        roster={uniqueKids(p.staticRows ?? [])}
        locks={new Set()}
        absences={new Set()}
        canEdit={false}
        stats={p.stats}
        seasonId={p.seasonId}
        startDate={null}
        scheduleId={null}
        actions={noActions}
        onResolve={() => {}}
      />
    );
  if (error) return <p className="err">⚠ {error}</p>;
  if (!data) return <p className="center-note">Rooster laden…</p>;
  return <Live key={p.scheduleId} {...p} scheduleId={p.scheduleId} data={data} />;
}

const noActions: BoatActions = { move() {}, swap() {}, toggleLock() {}, toggleAbsent() {} };

const uniqueKids = (rows: Row[]): Row[] => {
  const m = new Map<string, Row>();
  for (const r of rows) if (!m.has(r.name)) m.set(r.name, r);
  return [...m.values()];
};

function Live(p: Props & { scheduleId: number; data: ScheduleData }) {
  const { me } = useSession();
  const roster = useMemo(() => uniqueKids(p.data.rows), [p.data.rows]);
  const y = useYSchedule({
    scheduleId: p.scheduleId,
    initialRows: p.data.rows,
    constraints: p.data.constraints,
    canEdit: me.is_admin,
    userName: me.name ?? "instructeur",
  });

  const resolve = () => {
    const absences = [...y.absences].map((k) => {
      const i = k.lastIndexOf("|");
      return { name: y.canonical(k.slice(0, i)), day: k.slice(i + 1) };
    });
    const locks = y.rows
      .filter((r) => y.locks.has(cellKey(r.name, r.day)))
      .map((r) => ({ name: y.canonical(r.name), day: r.day, boat: r.boat }));
    p.onResolve({ absences, locks });
  };

  return (
    <Body
      rows={y.rows}
      roster={roster}
      locks={y.locks}
      absences={y.absences}
      canEdit={me.is_admin}
      stats={p.stats}
      seasonId={p.seasonId}
      startDate={p.data.start_date}
      scheduleId={p.scheduleId}
      actions={y}
      live={{ connected: y.connected, peers: y.peers, canUndo: y.canUndo, canRedo: y.canRedo, undo: y.undo, redo: y.redo }}
      onResolve={resolve}
    />
  );
}

interface BodyProps {
  rows: Row[];
  roster: Row[];
  locks: Set<string>;
  absences: Set<string>;
  canEdit: boolean;
  stats: Stats | null;
  seasonId: number | null;
  startDate: string | null;
  scheduleId: number | null;
  actions: BoatActions;
  live?: { connected: boolean; peers: string[]; canUndo: boolean; canRedo: boolean; undo: () => void; redo: () => void };
  onResolve: () => void;
}

const localToday = () => new Date().toLocaleDateString("sv-SE");

function Body(p: BodyProps) {
  const { cfg } = useSession();
  const toast = useToast();
  const [view, setView] = useState<"boat" | "kid">("boat");
  const [card, setCard] = useState<{ name: string; level: string } | null>(null);
  const [startDate, setStartDate] = useState(p.startDate);
  const [committing, setCommitting] = useState(false);

  const days = useMemo(() => sortDays([...new Set(p.rows.map((r) => r.day))]), [p.rows]);
  const levelRank = useMemo(() => Object.fromEntries(cfg.levels.map((l, i) => [l, i])), [cfg.levels]);

  const conflicts = useMemo(
    () =>
      computeConflicts({
        rows: p.rows,
        roster: p.roster.map((r) => r.name),
        absent: p.absences,
        boats: cfg.boats,
        restricted: new Set(cfg.restricted),
        levelRank,
        adj: cfg.adj ?? 1,
      }),
    [p.rows, p.roster, p.absences, cfg, levelRank],
  );
  const conflictCells = useMemo(
    () => new Set(conflicts.flatMap((c) => (c.kind === "dubbel" || c.kind === "afwezig" || c.kind === "niveau" ? c.names.map((n) => cellKey(n, c.day)) : []))),
    [conflicts],
  );
  const badBoats = useMemo(
    () => new Set(conflicts.filter((c) => c.boat !== undefined && (c.kind === "niveau" || c.kind === "bezetting")).map((c) => `${c.day}|${c.boat}`)),
    [conflicts],
  );

  const locked = startDate != null && startDate <= localToday();

  const commit = async () => {
    if (p.scheduleId == null) return;
    setCommitting(true);
    try {
      const r = await api<CommitResult>(`/schedule/${p.scheduleId}/commit`, { method: "POST" });
      if (r.ok) toast("Rooster vastgelegd ✓");
      else
        toast(
          `Niet vastgelegd: ${(r.conflicts ?? []).map((c) => `${c.name} op ${c.day} staat in meerdere boten`).join(", ")}`,
          "error",
        );
    } catch (e) {
      toast(`Vastleggen mislukt: ${errMsg(e)}`, "error");
    } finally {
      setCommitting(false);
    }
  };

  const saveStartDate = async (val: string) => {
    if (p.scheduleId == null) return;
    try {
      await api(`/schedule/${p.scheduleId}/start-date`, { json: { start_date: val || null } });
      setStartDate(val || null);
    } catch (e) {
      toast(`Startdatum instellen mislukt: ${errMsg(e)}`, "error");
    }
  };

  const exportCsv = () => {
    const lines = ["day,boat,name,level", ...p.rows.map((r) => [r.day, r.boat, `"${r.name.replace(/"/g, '""')}"`, r.level].join(","))];
    const url = URL.createObjectURL(new Blob([lines.join("\r\n")], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "rooster.csv";
    a.click();
    URL.revokeObjectURL(url);
  };

  const approx = p.stats && [p.stats.status_A, p.stats.status_B, p.stats.status_C].some((s) => s && s !== "OPTIMAL");

  return (
    <div>
      {p.stats && (
        <div className="summary">
          <div className="stat"><b>{p.stats.mixed}</b><span>gemengde boten</span></div>
          <div className="stat"><b>{p.stats.distinct}</b><span>distinct-niveau totaal</span></div>
          <div className="stat"><b>{p.stats.repeats}</b><span>herhaalde koppels</span></div>
        </div>
      )}
      {approx && (
        <p className="season-hint" role="status">
          De solver kon binnen de tijd niet bewijzen dat dit de beste indeling is.
        </p>
      )}

      <div className="controls">
        <div className="seg">
          <button aria-pressed={view === "boat"} onClick={() => setView("boat")}>Per Boot</button>
          <button aria-pressed={view === "kid"} onClick={() => setView("kid")}>Per Persoon</button>
        </div>
      </div>

      <div className="toolbar">
        {p.live && (
          <span className="presence-badge" title={p.live.peers.join(", ")}>
            <span className={`presence-dot${p.live.connected ? "" : " off"}`} />
            {p.live.connected ? `${p.live.peers.length} verbonden` : "Verbinding verbroken — wijzigingen worden later gesynchroniseerd"}
          </span>
        )}
        {!p.canEdit && p.scheduleId != null && <span className="badge restricted">Alleen-lezen · alleen beheerders</span>}
        {p.canEdit && locked && <span className="badge restricted">Startdatum verstreken</span>}
        {p.scheduleId != null && (
          <label style={{ display: "inline-flex", alignItems: "center", gap: 6, fontWeight: 700, fontSize: 14 }}>
            Startdatum
            <input type="date" className="sel" style={{ flex: "0 0 auto" }} value={startDate ?? ""} onChange={(e) => void saveStartDate(e.target.value)} />
          </label>
        )}
        <span className="spacer" />
        {p.live && p.canEdit && (
          <>
            <button className="btn-sm" onClick={p.live.undo} disabled={!p.live.canUndo}>↶ Ongedaan</button>
            <button className="btn-sm" onClick={p.live.redo} disabled={!p.live.canRedo}>↷ Opnieuw</button>
          </>
        )}
        <button className="btn-sm" onClick={exportCsv}>CSV</button>
        <button className="btn-sm" onClick={() => window.print()}>Afdrukken</button>
        {p.canEdit && p.scheduleId != null && (
          <>
            <button className="btn-sm" onClick={p.onResolve} title="Vergrendelde personen en afwezigheid blijven staan; de rest wordt opnieuw ingedeeld">
              Opnieuw indelen
            </button>
            <button className="btn-sm primary" onClick={() => void commit()} disabled={committing}>
              {committing ? "Vastleggen…" : "Rooster vastleggen"}
            </button>
          </>
        )}
      </div>

      {p.scheduleId != null && <ConflictPanel conflicts={conflicts} />}

      {p.scheduleId != null && (p.canEdit || p.absences.size > 0) && (
        <AbsenceEditor roster={p.roster} days={days} absences={p.absences} canEdit={p.canEdit} toggle={p.actions.toggleAbsent} />
      )}

      <div className="legend">
        {cfg.levels.map((l) => (
          <span key={l}><i className={`dot lv-${l}`} />Niveau {l}</span>
        ))}
        <span><i className="dot" style={{ background: "var(--mix)" }} />Gemengde boot</span>
      </div>

      {view === "boat" ? (
        <BoatView
          rows={p.rows}
          roster={p.roster}
          locks={p.locks}
          absences={p.absences}
          conflictCells={conflictCells}
          badBoats={badBoats}
          canEdit={p.canEdit && p.scheduleId != null}
          onOpenCard={(name, level) => p.seasonId != null && setCard({ name, level })}
          {...p.actions}
        />
      ) : (
        <KidView rows={p.rows} roster={p.roster} absences={p.absences} onOpenCard={(name, level) => p.seasonId != null && setCard({ name, level })} />
      )}

      {card && p.seasonId != null && (
        <EvalCard name={card.name} level={card.level} seasonId={p.seasonId} scheduleDays={days} onClose={() => setCard(null)} />
      )}
    </div>
  );
}
