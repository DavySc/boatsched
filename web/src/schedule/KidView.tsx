import type { Row } from "../types";
import { useSession } from "../session";
import { dayName, sortDays } from "../i18n";
import { cellKey, pairCounts, pairKey } from "./conflicts";

interface Props {
  rows: Row[];
  roster: Row[];
  absences: Set<string>;
  onOpenCard: (name: string, level: string) => void;
}

export function KidView({ rows, roster, absences, onOpenCard }: Props) {
  const { cfg } = useSession();
  const rank = (l: string) => cfg.levels.indexOf(l) < 0 ? 99 : cfg.levels.indexOf(l);
  const days = sortDays([...new Set(rows.map((r) => r.day))]);
  const kids = [...roster].sort((a, b) => rank(a.level) - rank(b.level) || a.name.localeCompare(b.name));
  const counts = pairCounts(rows);
  const maxCount = counts.size ? Math.max(...counts.values()) : 0;

  return (
    <section className="day">
      <div className="day-head">
        <h3>Samenstelling per persoon</h3>
        {maxCount >= 2 && <span className="badge restricted">top koppel: {maxCount} dagen</span>}
        <span className="meta">gesorteerd op niveau</span>
      </div>
      <div className="kid-table">
        <table>
          <thead>
            <tr>
              <th>Persoon</th>
              {days.map((d) => (
                <th key={d}>{dayName(d)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {kids.map((k) => (
              <tr key={k.name}>
                <td className="name-cell">
                  <span className={`lv lv-${k.level}`}>{k.level}</span>
                  <button className="name-btn" onClick={() => onOpenCard(k.name, k.level)}>
                    {k.name}
                  </button>
                </td>
                {days.map((d) => {
                  if (absences.has(cellKey(k.name, d))) return <td key={d}><span className="pill absent">afwezig</span></td>;
                  const mine = rows.find((r) => r.name === k.name && r.day === d);
                  if (!mine) return <td key={d}>—</td>;
                  const mates = rows
                    .filter((r) => r.day === d && r.boat === mine.boat && r.name !== k.name)
                    .sort((a, b) => rank(a.level) - rank(b.level) || a.name.localeCompare(b.name));
                  return (
                    <td key={d}>
                      <span className="pill">⚓ {mine.boat}</span>{" "}
                      {mates.map((m) => {
                        const cnt = counts.get(pairKey(k.name, m.name)) ?? 0;
                        const top = maxCount >= 2 && cnt === maxCount;
                        return (
                          <span key={m.name} className={`pill${top ? " top-pair" : ""}`}>
                            {m.name}
                            {top && <sup className="pair-cnt">{cnt}</sup>}
                          </span>
                        );
                      })}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
