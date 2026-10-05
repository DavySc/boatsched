import { useEffect, useRef, useState } from "react";
import { api, errMsg } from "../api";
import { DAY_NL_SHORT } from "../i18n";
import type { DayProgress, Instructor, Member, SkillCatalog, StudentProgress } from "../types";

const STD_CODES = ["", "attempted", "completed"];
const OVR_CODES = ["", "O", "T", "V", "G"];
const GLYPH: Record<string, string> = { attempted: "•", completed: "✓", O: "O", T: "T", V: "V", G: "G" };
const WEEKDAYS = new Set(["Fri", "Sat", "Sun", "Mon", "Tue", "Wed"]);
const CODE_LABEL: Record<string, string> = { attempted: "geprobeerd", completed: "behaald" };

interface Props {
  name: string;
  level: string;
  seasonId: number;
  scheduleDays: string[];
  onClose: () => void;
}

interface Loaded {
  catalog: SkillCatalog;
  instructors: Instructor[];
  memberId: number | null;
  progress: StudentProgress;
}

export function EvalCard({ name, level, seasonId, scheduleDays, onClose }: Props) {
  const [data, setData] = useState<Loaded | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const timer = useRef<number | undefined>(undefined);
  const latest = useRef<Loaded | null>(null);
  latest.current = data;

  const days = scheduleDays.length ? scheduleDays.slice(0, 5) : ["Dag 1", "Dag 2", "Dag 3", "Dag 4", "Dag 5"];

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [cat, instructors, members, prog] = await Promise.all([
          api<{ levels: SkillCatalog }>("/api/skills"),
          api<Instructor[]>("/instructors"),
          api<Member[]>("/api/members"),
          api<{ progressData: (StudentProgress & { name?: string })[] }>(`/api/progress?season_id=${seasonId}`),
        ]);
        if (cancelled) return;
        const memberId = members.find((m) => m.name.toLowerCase() === name.toLowerCase())?.id ?? null;
        const mine = prog.progressData.find((p) => p.name?.toLowerCase() === name.toLowerCase());
        setData({
          catalog: cat.levels ?? {},
          instructors,
          memberId,
          progress: { member_id: memberId ?? -1, days: mine?.days ?? {}, notes: mine?.notes ?? "" },
        });
      } catch (e) {
        if (!cancelled) setError(errMsg(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [name, seasonId]);

  const save = async (silent: boolean) => {
    const cur = latest.current;
    if (!cur) return;
    if (cur.memberId == null) return setStatus("Onbekende deelnemer");
    if (!silent) setStatus("Opslaan…");
    const payload = {
      member_id: cur.memberId,
      season_id: seasonId,
      notes: cur.progress.notes,
      days: Object.entries(cur.progress.days)
        .filter(([day]) => WEEKDAYS.has(day))
        .map(([day, d]) => ({ day, skippers: [...new Set(d.skippers ?? [])], records: d.records ?? {} })),
    };
    try {
      await api("/api/progress", { json: payload });
      setStatus("Opgeslagen ✓");
      window.setTimeout(() => setStatus(""), 2000);
    } catch (e) {
      setStatus(`Opslaan mislukt: ${errMsg(e)}`);
    }
  };

  const change = (fn: (p: StudentProgress) => void) => {
    setData((d) => {
      if (!d) return d;
      const progress = structuredClone(d.progress);
      fn(progress);
      return { ...d, progress };
    });
    setStatus("Niet opgeslagen…");
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void save(true), 1800);
  };

  // Flush a pending autosave when the card is closed.
  const close = () => {
    if (timer.current) {
      window.clearTimeout(timer.current);
      void save(true);
    }
    onClose();
  };

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    document.addEventListener("keydown", esc);
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", esc);
      document.body.style.overflow = "";
    };
  });

  const ensureDay = (p: StudentProgress, day: string): DayProgress => (p.days[day] ??= { skippers: [], records: {} });

  const cycle = (skillId: number, day: string, overall: boolean) =>
    change((p) => {
      const d = ensureDay(p, day);
      const codes = overall ? OVR_CODES : STD_CODES;
      const cur = d.records[skillId] ?? "";
      const next = codes[(codes.indexOf(cur) + 1) % codes.length];
      if (next) d.records[skillId] = next;
      else delete d.records[skillId];
    });

  const skipperSelect = (day: string, pos: 0 | 1) => (
    <select
      className="skipper-sel"
      aria-label={`Schipper ${pos + 1} op ${DAY_NL_SHORT[day] ?? day}`}
      value={data?.progress.days[day]?.skippers?.[pos] ?? ""}
      onChange={(e) =>
        change((p) => {
          const d = ensureDay(p, day);
          const ids = [d.skippers[0], d.skippers[1]];
          ids[pos] = e.target.value === "" ? (undefined as unknown as number) : Number(e.target.value);
          d.skippers = [...new Set(ids.filter((x) => x != null))];
        })
      }
    >
      <option value="">—</option>
      {data?.instructors.map((i) => (
        <option key={i.id} value={i.id}>
          {i.name}
        </option>
      ))}
    </select>
  );

  const cell = (skillId: number, day: string, overall: boolean) => {
    const code = data?.progress.days[day]?.records[skillId] ?? "";
    return (
      <td key={day} className={code ? `state-${code}` : ""}>
        <button
          className="cell-btn"
          aria-label={`${DAY_NL_SHORT[day] ?? day}: ${CODE_LABEL[code] ?? (code || "leeg")}`}
          onClick={() => cycle(skillId, day, overall)}
        >
          {GLYPH[code] ?? <span style={{ opacity: 0.22 }}>·</span>}
        </button>
      </td>
    );
  };

  return (
    <div className="eval-overlay" role="dialog" aria-modal="true" aria-label={`Beoordelingskaart ${name}`}>
      <div className="eval-wrap">
        <div className="eval-hdr-bar">
          <button className="btn-sm" onClick={close} autoFocus>
            ← Terug
          </button>
          <span className={`lv lv-${level}`} style={{ width: "auto", padding: "0 10px", height: 28 }}>
            {level}
          </span>
          <span className="eval-name">{name}</span>
          <span style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 10 }}>
            <span className="eval-save-status" role="status">
              {status}
            </span>
            <button className="btn-sm" onClick={() => void save(false)}>
              Opslaan
            </button>
          </span>
        </div>
        {error && <p className="err">⚠ {error}</p>}
        {!data && !error && <p className="center-note">Laden…</p>}
        {data && (
          <>
            <div className="eval-table-outer">
              <table className="eval-table">
                <thead>
                  <tr>
                    <th className="skill-col">Vaardigheid</th>
                    {days.map((d, i) => (
                      <th key={d}>
                        {DAY_NL_SHORT[d] ?? d}
                        <span style={{ display: "block", fontSize: 9, fontWeight: 400, opacity: 0.6 }}>DAG {i + 1}</span>
                      </th>
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
                  {(data.catalog[level] ?? []).flatMap((cat) => [
                    <tr key={`c-${cat.category}`}>
                      <td className="cat-hdr" colSpan={days.length + 1}>
                        {cat.category}
                      </td>
                    </tr>,
                    ...cat.skills.flatMap((s) => [
                      <tr key={`s-${s.id}`}>
                        <td className="skill-label">{s.label}</td>
                        {days.map((d) => cell(s.id, d, cat.eval_type === "overall"))}
                      </tr>,
                      ...(s.subskills ?? []).map((sub) => (
                        <tr key={`ss-${sub.id}`}>
                          <td className="skill-label sub">{sub.label}</td>
                          {days.map((d) => cell(sub.id, d, cat.eval_type === "overall"))}
                        </tr>
                      )),
                    ]),
                  ])}
                </tbody>
              </table>
            </div>
            <div className="eval-notes-section">
              <label className="login-label" htmlFor="eval-notes">
                Notities
              </label>
              <textarea
                id="eval-notes"
                className="eval-notes"
                placeholder="Vrije notities voor deze deelnemer…"
                value={data.progress.notes}
                onChange={(e) => change((p) => void (p.notes = e.target.value))}
              />
            </div>
          </>
        )}
      </div>
    </div>
  );
}
