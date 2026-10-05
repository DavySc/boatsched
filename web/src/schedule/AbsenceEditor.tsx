import type { Row } from "../types";
import { dayName, sortDays } from "../i18n";
import { cellKey } from "./conflicts";

interface Props {
  roster: Row[];
  days: string[];
  absences: Set<string>;
  canEdit: boolean;
  toggle: (name: string, day: string) => void;
}

/** Who is away on which day. Marked cells are left out when the schedule is re-solved. */
export function AbsenceEditor({ roster, days, absences, canEdit, toggle }: Props) {
  const cols = sortDays(days);
  return (
    <details className="absence-box">
      <summary>
        <strong>Afwezigheid</strong> ({absences.size} {absences.size === 1 ? "markering" : "markeringen"})
      </summary>
      <p style={{ margin: "8px 0", fontSize: 13, color: "var(--muted)" }}>
        Een afwezig kind haalt u uit de boot en blijft bij opnieuw indelen buiten de boten.
      </p>
      <table className="absence-table">
        <thead>
          <tr>
            <th>Naam</th>
            {cols.map((d) => (
              <th key={d}>{dayName(d).slice(0, 3)}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {roster.map((k) => (
            <tr key={k.name}>
              <td>{k.name}</td>
              {cols.map((d) => {
                const on = absences.has(cellKey(k.name, d));
                return (
                  <td key={d}>
                    <button
                      aria-pressed={on}
                      aria-label={`${k.name} afwezig op ${dayName(d)}`}
                      disabled={!canEdit}
                      onClick={() => toggle(k.name, d)}
                    >
                      {on ? "✕" : ""}
                    </button>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </details>
  );
}
