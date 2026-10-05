import type { Row } from "../types";
import { dayName, sortDays } from "../i18n";

export const cellKey = (name: string, day: string) => `${name.trim().toLowerCase()}|${day}`;

export type ConflictKind = "dubbel" | "niet ingedeeld" | "afwezig" | "niveau" | "bezetting";

export interface Conflict {
  kind: ConflictKind;
  day: string;
  boat?: number;
  names: string[];
  text: string;
}

export interface ConflictInput {
  rows: Row[];
  /** Display names of everybody who belongs in the schedule. */
  roster: string[];
  /** `cellKey(name, day)` of every absence. */
  absent: Set<string>;
  boats: number;
  restricted: Set<string>;
  levelRank: Record<string, number>;
  adj: number;
}

/**
 * Everything that makes the current plan invalid or lopsided. The same rules the
 * solver enforces, so a hand edit is flagged the moment it breaks one.
 */
export function computeConflicts(inp: ConflictInput): Conflict[] {
  const out: Conflict[] = [];
  const days = sortDays([...new Set(inp.rows.map((r) => r.day))]);
  for (const day of days) {
    const dayRows = inp.rows.filter((r) => r.day === day);
    const placed = new Map<string, Row[]>();
    for (const r of dayRows) {
      const k = cellKey(r.name, day);
      placed.set(k, [...(placed.get(k) ?? []), r]);
    }
    let present = 0;
    for (const name of inp.roster) {
      const k = cellKey(name, day);
      const where = placed.get(k) ?? [];
      if (inp.absent.has(k)) {
        if (where.length)
          out.push({ kind: "afwezig", day, boat: where[0].boat, names: [name], text: `${name} is afwezig op ${dayName(day)} maar staat in een boot` });
        continue;
      }
      present++;
      if (where.length === 0)
        out.push({ kind: "niet ingedeeld", day, names: [name], text: `${name} heeft geen boot op ${dayName(day)}` });
      else if (new Set(where.map((r) => r.boat)).size > 1)
        out.push({ kind: "dubbel", day, names: [name], text: `${name} staat op ${dayName(day)} in boten ${where.map((r) => r.boat).join(" en ")}` });
    }

    if (inp.restricted.has(day)) {
      for (const boat of new Set(dayRows.map((r) => r.boat))) {
        const ranks = dayRows.filter((r) => r.boat === boat).map((r) => inp.levelRank[r.level] ?? 0);
        if (ranks.length && Math.max(...ranks) - Math.min(...ranks) > inp.adj)
          out.push({ kind: "niveau", day, boat, names: dayRows.filter((r) => r.boat === boat).map((r) => r.name), text: `Boot ${boat} op ${dayName(day)} mengt niveaus die te ver uit elkaar liggen` });
      }
    }

    if (inp.boats > 0) {
      const lo = Math.floor(present / inp.boats);
      const hi = lo + (present % inp.boats ? 1 : 0);
      for (let b = 1; b <= inp.boats; b++) {
        const n = dayRows.filter((r) => r.boat === b).length;
        if (n < lo || n > hi)
          out.push({ kind: "bezetting", day, boat: b, names: [], text: `Boot ${b} op ${dayName(day)} heeft ${n} personen (verwacht ${lo === hi ? lo : `${lo}–${hi}`})` });
      }
    }
  }
  return out;
}

/** How often each pair of people shares a boat, keyed `a\0b` with a < b. */
export function pairCounts(rows: Row[]): Map<string, number> {
  const groups = new Map<string, string[]>();
  for (const r of rows) {
    const k = `${r.day}\0${r.boat}`;
    groups.set(k, [...(groups.get(k) ?? []), r.name]);
  }
  const counts = new Map<string, number>();
  for (const members of groups.values())
    for (let i = 0; i < members.length; i++)
      for (let j = i + 1; j < members.length; j++) {
        const k = pairKey(members[i], members[j]);
        counts.set(k, (counts.get(k) ?? 0) + 1);
      }
  return counts;
}

export const pairKey = (a: string, b: string) => (a < b ? `${a}\0${b}` : `${b}\0${a}`);
