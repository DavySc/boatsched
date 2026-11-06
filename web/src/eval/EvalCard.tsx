import { useEffect, useState, useSyncExternalStore } from "react";
import { api, errMsg } from "../api";
import { DAY_NL, DAY_NL_SHORT, DAY_ORDER } from "../i18n";
import type { DayProgress, Instructor, Member, Row, SkillCatalog, StudentProgress } from "../types";
import { enqueue, flush, pendingFor, subscribe, syncStatus, type Change } from "./outbox";

const STD_CODES = ["", "attempted", "completed"];
const OVR_CODES = ["", "O", "T", "V", "G"];
const GLYPH: Record<string, string> = { attempted: "•", completed: "✓", O: "O", T: "T", V: "V", G: "G" };
const CODE_LABEL: Record<string, string> = { "": "—", attempted: "Geprobeerd", completed: "Behaald", O: "O", T: "T", V: "V", G: "G" };
// The days the server accepts evaluations for (see eval_assessment's CHECK).
const EVAL_DAYS = new Set(DAY_ORDER);
const WEEKDAY = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const narrow = () => matchMedia("(max-width: 700px)").matches;

interface Props {
  name: string;
  level: string;
  seasonId: number;
  scheduleDays: string[];
  /** Schedule rows, for stepping through the kid's boat-mates. */
  rows: Row[];
  onOpen: (name: string, level: string) => void;
  onClose: () => void;
}

interface Shared {
  catalog: SkillCatalog;
  instructors: Instructor[];
  members: Member[];
}

// Catalog, instructors and members rarely change: fetch them once, not per card.
let shared: Promise<Shared> | null = null;
const loadShared = () =>
  (shared ??= Promise.all([
    api<{ levels: SkillCatalog }>("/api/skills"),
    api<Instructor[]>("/instructors"),
    api<Member[]>("/api/members"),
  ]).then(([cat, instructors, members]) => ({ catalog: cat.levels ?? {}, instructors, members }))).catch((e) => {
    shared = null;
    throw e;
  });

const applyChange = (p: StudentProgress, c: Change) => {
  if (c.kind === "notes") {
    p.notes = c.text;
    return;
  }
  const d: DayProgress = (p.days[c.day] ??= { skippers: [], records: {} });
  if (c.kind === "skippers") d.skippers = c.ids;
  else if (c.status) d.records[c.skill_id] = c.status;
  else delete d.records[c.skill_id];
};

export function EvalCard({ name, level, seasonId, scheduleDays, rows, onOpen, onClose }: Props) {
  const [base, setBase] = useState<Shared | null>(null);
  const [progress, setProgress] = useState<StudentProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const sync = useSyncExternalStore(subscribe, () => JSON.stringify(syncStatus()));
  const { state, pending, lastError } = JSON.parse(sync) as ReturnType<typeof syncStatus>;

  const days = (scheduleDays.length ? scheduleDays : DAY_ORDER).filter((d) => EVAL_DAYS.has(d));
  const [shown, setShown] = useState<string>(() => {
    if (!narrow()) return "week";
    const today = WEEKDAY[new Date().getDay()];
    return days.includes(today) ? today : days[0];
  });

  const memberId = base?.members.find((m) => m.name.toLowerCase() === name.toLowerCase())?.id ?? null;

  useEffect(() => {
    let cancelled = false;
    setProgress(null);
    setError(null);
    (async () => {
      try {
        const s = await loadShared();
        if (cancelled) return;
        setBase(s);
        const id = s.members.find((m) => m.name.toLowerCase() === name.toLowerCase())?.id;
        if (id == null) return setError("Onbekende deelnemer");
        const prog = await api<{ progressData: StudentProgress[] }>(`/api/progress?season_id=${seasonId}&member_id=${id}`);
        if (cancelled) return;
        const mine = prog.progressData[0];
        const p: StudentProgress = { member_id: id, days: mine?.days ?? {}, notes: mine?.notes ?? "" };
        // Edits not yet on the server (e.g. made offline) win over what it sent.
        for (const c of pendingFor(id, seasonId)) applyChange(p, c);
        setProgress(p);
      } catch (e) {
        if (!cancelled) setError(errMsg(e));
      }
    })();
    void flush();
    return () => {
      cancelled = true;
    };
  }, [name, seasonId]);

  // Keep the offline copy of this card current, so reopening it without
  // signal shows the latest taps, not the state when it was last fetched.
  // The cache name must match rust/crates/server/static/sw.js.
  useEffect(() => {
    if (!progress || !("caches" in window)) return;
    const url = `/api/progress?season_id=${seasonId}&member_id=${progress.member_id}`;
    const body = JSON.stringify({ progressData: [progress] });
    void caches
      .open("boatsched-v1")
      .then((c) => c.put(url, new Response(body, { headers: { "Content-Type": "application/json" } })))
      .catch(() => {});
  }, [progress, seasonId]);

  const change = (c: Change) => {
    if (memberId == null) return;
    setProgress((p) => {
      if (!p) return p;
      const next = structuredClone(p);
      applyChange(next, c);
      return next;
    });
    enqueue(memberId, seasonId, c);
    navigator.vibrate?.(8);
  };

  const setCell = (skillId: number, day: string, status: string) => change({ kind: "cell", day, skill_id: skillId, status });
  const codeOf = (skillId: number, day: string) => progress?.days[day]?.records[skillId] ?? "";

  const setSkipper = (day: string, pos: 0 | 1, value: string) => {
    const cur = progress?.days[day]?.skippers ?? [];
    const ids: (number | undefined)[] = [cur[0], cur[1]];
    ids[pos] = value === "" ? undefined : Number(value);
    change({ kind: "skippers", day, ids: [...new Set(ids.filter((x): x is number => x != null))] });
  };

  // Boat-mates on the shown day (or the first day in week view), for prev/next.
  const navDay = shown === "week" ? days[0] : shown;
  const mine = rows.find((r) => r.day === navDay && r.name === name);
  const crew = mine ? rows.filter((r) => r.day === navDay && r.boat === mine.boat).sort((a, b) => a.name.localeCompare(b.name)) : [];
  const at = crew.findIndex((r) => r.name === name);
  const step = (delta: number) => {
    const r = crew[(at + delta + crew.length) % crew.length];
    if (r) onOpen(r.name, r.level);
  };

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", esc);
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", esc);
      document.body.style.overflow = "";
    };
  }, [onClose]);

  const syncText =
    state === "sending"
      ? "Versturen…"
      : pending === 0
        ? lastError
          ? `Niet opgeslagen: ${lastError}`
          : "Opgeslagen ✓"
        : state === "offline"
          ? `Offline · ${pending} ${pending === 1 ? "wijziging wacht" : "wijzigingen wachten"}`
          : `${pending} niet verstuurd${lastError ? ` · ${lastError}` : ""}`;

  const skipperSelect = (day: string, pos: 0 | 1) => (
    <select
      className="skipper-sel"
      aria-label={`Schipper ${pos + 1} op ${DAY_NL[day] ?? day}`}
      value={progress?.days[day]?.skippers?.[pos] ?? ""}
      onChange={(e) => setSkipper(day, pos, e.target.value)}
    >
      <option value="">{pos === 0 ? "Schipper…" : "2e schipper…"}</option>
      {base?.instructors.map((i) => (
        <option key={i.id} value={i.id}>
          {i.name}
        </option>
      ))}
    </select>
  );

  const catalog = base?.catalog[level] ?? [];

  // Week grid: tap a cell to cycle through the states.
  const weekCell = (skillId: number, day: string, overall: boolean) => {
    const code = codeOf(skillId, day);
    const codes = overall ? OVR_CODES : STD_CODES;
    return (
      <td key={day} className={code ? `state-${code}` : ""}>
        <button
          className="cell-btn"
          aria-label={`${DAY_NL[day] ?? day}: ${code ? CODE_LABEL[code] : "leeg"}`}
          onClick={() => setCell(skillId, day, codes[(codes.indexOf(code) + 1) % codes.length])}
        >
          {GLYPH[code] ?? <span style={{ opacity: 0.22 }}>·</span>}
        </button>
      </td>
    );
  };

  const week = (
    <div className="eval-table-outer">
      <table className="eval-table">
        <thead>
          <tr>
            <th className="skill-col">Vaardigheid</th>
            {days.map((d) => (
              <th key={d}>{DAY_NL_SHORT[d] ?? d}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          <tr>
            <td className="skill-label" style={{ fontStyle: "italic", fontWeight: 400 }}>
              Schipper
            </td>
            {days.map((d) => (
              <td key={d} style={{ padding: "4px 6px" }}>
                {skipperSelect(d, 0)}
                {skipperSelect(d, 1)}
              </td>
            ))}
          </tr>
          {catalog.flatMap((cat) => [
            <tr key={`c-${cat.category}`}>
              <td className="cat-hdr" colSpan={days.length + 1}>
                {cat.category}
              </td>
            </tr>,
            ...cat.skills.flatMap((s) => [
              <tr key={`s-${s.id}`}>
                <td className="skill-label">{s.label}</td>
                {days.map((d) => weekCell(s.id, d, cat.eval_type === "overall"))}
              </tr>,
              ...(s.subskills ?? []).map((sub) => (
                <tr key={`ss-${sub.id}`}>
                  <td className="skill-label sub">{sub.label}</td>
                  {days.map((d) => weekCell(sub.id, d, cat.eval_type === "overall"))}
                </tr>
              )),
            ]),
          ])}
        </tbody>
      </table>
    </div>
  );

  // One day: every skill gets explicit choices, so a single tap sets the value
  // instead of cycling past it.
  const choice = (skillId: number, label: string, sub: boolean, overall: boolean) => {
    const code = codeOf(skillId, shown);
    return (
      <li key={skillId} className={`skill-row${sub ? " sub" : ""}`}>
        <span className="skill-name">{label}</span>
        <span className="choice" role="group" aria-label={label}>
          {(overall ? OVR_CODES : STD_CODES).slice(1).map((c) => (
            <button
              key={c}
              className={`state-${c}`}
              aria-pressed={code === c}
              onClick={() => setCell(skillId, shown, code === c ? "" : c)}
            >
              {CODE_LABEL[c]}
            </button>
          ))}
        </span>
      </li>
    );
  };

  const day = (
    <div className="eval-day">
      <div className="eval-skippers">
        {skipperSelect(shown, 0)}
        {skipperSelect(shown, 1)}
      </div>
      {catalog.map((cat) => (
        <section key={cat.category} className="skill-group">
          <h4>{cat.category}</h4>
          <ul>
            {cat.skills.flatMap((s) => [
              choice(s.id, s.label, false, cat.eval_type === "overall"),
              ...(s.subskills ?? []).map((sub) => choice(sub.id, sub.label, true, cat.eval_type === "overall")),
            ])}
          </ul>
        </section>
      ))}
    </div>
  );

  return (
    <div className="eval-overlay" role="dialog" aria-modal="true" aria-label={`Beoordelingskaart ${name}`}>
      <div className="eval-wrap">
        <div className="eval-hdr-bar">
          <button className="btn-sm" onClick={onClose} autoFocus>
            ← Terug
          </button>
          <span className={`lv lv-${level}`}>{level}</span>
          <span className="eval-name">{name}</span>
          <span className={`eval-save-status${pending ? " pending" : ""}`} role="status">
            {syncText}
          </span>
        </div>
        <div className="eval-days seg" role="group" aria-label="Dag">
          {days.map((d) => (
            <button key={d} aria-pressed={shown === d} aria-label={DAY_NL[d] ?? d} onClick={() => setShown(d)}>
              {DAY_NL_SHORT[d] ?? d}
            </button>
          ))}
          <button aria-pressed={shown === "week"} onClick={() => setShown("week")}>
            Week
          </button>
        </div>
        {error && <p className="err">⚠ {error}</p>}
        {!progress && !error && <p className="center-note">Laden…</p>}
        {progress && (
          <>
            {shown === "week" ? week : day}
            <div className="eval-notes-section">
              <label className="login-label" htmlFor="eval-notes">
                Notities
              </label>
              <textarea
                id="eval-notes"
                className="eval-notes"
                placeholder="Vrije notities voor deze deelnemer…"
                value={progress.notes}
                onChange={(e) => change({ kind: "notes", text: e.target.value })}
              />
            </div>
          </>
        )}
        {crew.length > 1 && (
          <nav className="eval-nav" aria-label="Andere bemanning">
            <button className="btn-sm" onClick={() => step(-1)}>
              ‹ {crew[(at - 1 + crew.length) % crew.length].name}
            </button>
            <span className="eval-nav-pos">
              Boot {mine?.boat} · {at + 1}/{crew.length}
            </span>
            <button className="btn-sm" onClick={() => step(1)}>
              {crew[(at + 1) % crew.length].name} ›
            </button>
          </nav>
        )}
      </div>
    </div>
  );
}
