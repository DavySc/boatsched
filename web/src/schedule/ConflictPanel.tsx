import type { Conflict } from "./conflicts";

export function ConflictPanel({ conflicts }: { conflicts: Conflict[] }) {
  if (conflicts.length === 0)
    return (
      <div className="conflicts clean" role="status">
        ✓ Geen conflicten: iedereen heeft elke dag een boot, niveaus en bezetting kloppen.
      </div>
    );
  return (
    <details className="conflicts" open={conflicts.length <= 6}>
      <summary>
        {conflicts.length} {conflicts.length === 1 ? "punt" : "punten"} om te controleren
      </summary>
      <ul>
        {conflicts.map((c, i) => (
          <li key={i}>
            <span className="kind">{c.kind}</span>
            {c.text}
          </li>
        ))}
      </ul>
    </details>
  );
}
