import { ApiError, api } from "../api";

// Evaluation edits are written here first and sent when the server can be
// reached, so taps made on the water without signal are never lost. Kept in
// localStorage so they survive closing the app, a reload, or a re-login.

export type Change =
  | { kind: "cell"; day: string; skill_id: number; status: string }
  | { kind: "skippers"; day: string; ids: number[] }
  | { kind: "notes"; text: string };

interface Entry {
  member_id: number;
  season_id: number;
  change: Change;
}

const KEY = "bs-outbox";
const RETRY_MS = 20_000;

const read = (): Entry[] => {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "[]") as Entry[];
  } catch {
    return [];
  }
};
const write = (q: Entry[]) => localStorage.setItem(KEY, JSON.stringify(q));

// A later edit of the same cell / day's skippers / notes replaces the earlier one.
const target = (e: Entry) => {
  const c = e.change;
  const where = c.kind === "cell" ? `${c.day}:${c.skill_id}` : c.kind === "skippers" ? c.day : "";
  return `${e.member_id}|${e.season_id}|${c.kind}|${where}`;
};

export type SyncState = "idle" | "sending" | "offline" | "error";
let state: SyncState = "idle";
let lastError = "";
const listeners = new Set<() => void>();
const notify = () => listeners.forEach((l) => l());

export function subscribe(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export const syncStatus = () => ({ state, lastError, pending: read().length });

export const pendingFor = (memberId: number, seasonId: number): Change[] =>
  read()
    .filter((e) => e.member_id === memberId && e.season_id === seasonId)
    .map((e) => e.change);

let timer: number | undefined;

export function enqueue(memberId: number, seasonId: number, change: Change) {
  const entry = { member_id: memberId, season_id: seasonId, change };
  const t = target(entry);
  write([...read().filter((e) => target(e) !== t), entry]);
  notify();
  window.clearTimeout(timer);
  timer = window.setTimeout(() => void flush(), 800);
}

let flushing = false;

/** Send everything queued. Safe to call any time; concurrent calls coalesce. */
export async function flush(): Promise<void> {
  if (flushing) return;
  const sent = read();
  if (!sent.length) return;
  flushing = true;
  state = "sending";
  lastError = "";
  notify();
  try {
    const groups = new Map<string, Entry[]>();
    for (const e of sent) {
      const k = `${e.member_id}|${e.season_id}`;
      groups.set(k, [...(groups.get(k) ?? []), e]);
    }
    for (const entries of groups.values()) {
      const { member_id, season_id } = entries[0];
      try {
        await api("/api/progress/changes", { json: { member_id, season_id, changes: entries.map((e) => e.change) } });
      } catch (e) {
        // A 400 means the server will never accept these (e.g. the kid was
        // removed); drop them rather than block everything queued behind them.
        if (!(e instanceof ApiError && e.status === 400)) throw e;
        lastError = e.message;
      }
      // Remove exactly what was sent; edits made meanwhile stay queued.
      const done = new Set(entries.map((e) => JSON.stringify(e)));
      write(read().filter((e) => !done.has(JSON.stringify(e))));
    }
    state = lastError ? "error" : "idle";
  } catch (e) {
    // Offline or logged out: keep everything and try again later. After a
    // re-login the next retry picks it up.
    state = e instanceof ApiError && e.status === 0 ? "offline" : "error";
    lastError = e instanceof Error ? e.message : String(e);
  } finally {
    flushing = false;
    notify();
  }
  // Edits made while sending go out right away instead of on the next retry.
  if (state === "idle" && read().length) void flush();
}

window.addEventListener("online", () => void flush());
document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && void flush());
window.setInterval(() => void flush(), RETRY_MS);
