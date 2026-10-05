import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import * as Y from "yjs";
import { WebsocketProvider } from "y-websocket";
import type { Constraints, Row } from "../types";
import { cellKey } from "./conflicts";

const ORIGIN = "local";

interface Options {
  scheduleId: number;
  initialRows: Row[];
  constraints: Constraints;
  canEdit: boolean;
  userName: string;
}

const COLORS = ["#c2522d", "#1f7a8c", "#5b8c5a", "#d99a2b", "#7a3b69", "#a8412a"];
const colorFor = (s: string) => COLORS[[...s].reduce((a, c) => a + c.charCodeAt(0), 0) % COLORS.length];

/**
 * Live schedule: a Yjs doc synced through `/collab/{id}`.
 *
 * - map `schedule`: `"{day}:{boat}"` → array of names (what the server commits)
 * - map `locks` / `absences`: `cellKey(name, day)` → true (planning inputs for re-solve)
 *
 * The doc stores names only, so levels are re-attached from `initialRows`.
 */
export function useYSchedule({ scheduleId, initialRows, constraints, canEdit, userName }: Options) {
  const [rows, setRows] = useState<Row[]>(initialRows);
  const [locks, setLocks] = useState<Set<string>>(new Set());
  const [absences, setAbsences] = useState<Set<string>>(new Set());
  const [connected, setConnected] = useState(false);
  const [peers, setPeers] = useState<string[]>([]);
  const [history, setHistory] = useState({ canUndo: false, canRedo: false });

  const doc = useRef<Y.Doc | null>(null);
  const undo = useRef<Y.UndoManager | null>(null);
  const rowsRef = useRef<Row[]>(initialRows);
  rowsRef.current = rows;

  // Canonical names + levels, case-insensitive (the doc and the DB can differ in case).
  const known = useMemo(() => {
    const m = new Map<string, Row>();
    for (const r of initialRows) m.set(r.name.trim().toLowerCase(), r);
    return m;
  }, [initialRows]);
  const canonical = useCallback((name: string) => known.get(name.trim().toLowerCase())?.name ?? name, [known]);
  const levelOf = useCallback((name: string) => known.get(name.trim().toLowerCase())?.level ?? "", [known]);

  useEffect(() => {
    const ydoc = new Y.Doc();
    doc.current = ydoc;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    const provider = new WebsocketProvider(`${proto}://${location.host}/collab`, String(scheduleId), ydoc);
    const ySchedule = ydoc.getMap<Y.Array<string>>("schedule");
    const yLocks = ydoc.getMap<boolean>("locks");
    const yAbs = ydoc.getMap<boolean>("absences");
    const manager = new Y.UndoManager([ySchedule, yLocks, yAbs], { trackedOrigins: new Set([ORIGIN]) });
    undo.current = manager;

    const syncRows = () => {
      const next: Row[] = [];
      for (const [key, arr] of ySchedule.entries()) {
        const colon = key.indexOf(":");
        if (colon < 0 || key.startsWith("_")) continue;
        const day = key.slice(0, colon);
        const boat = parseInt(key.slice(colon + 1), 10);
        for (const name of arr.toArray()) next.push({ day, boat, name, level: levelOf(name) });
      }
      // An empty doc means "not synced yet": keep what the REST call gave us.
      if (next.length) setRows(next);
    };
    const syncSets = () => {
      setLocks(new Set(yLocks.keys()));
      setAbsences(new Set(yAbs.keys()));
    };
    const syncHistory = () =>
      setHistory({ canUndo: manager.undoStack.length > 0, canRedo: manager.redoStack.length > 0 });

    ySchedule.observeDeep(syncRows);
    yLocks.observe(syncSets);
    yAbs.observe(syncSets);
    manager.on("stack-item-added", syncHistory);
    manager.on("stack-item-popped", syncHistory);

    provider.on("status", ({ status }: { status: string }) => setConnected(status === "connected"));
    provider.on("sync", (synced: boolean) => {
      if (!synced) return;
      syncRows();
      // First open of a schedule that was solved with absences/locks: seed the doc
      // from what the server stored with it.
      if (canEdit && yLocks.size === 0 && yAbs.size === 0) {
        ydoc.transact(() => {
          for (const a of constraints.absences) yAbs.set(cellKey(a.name, a.day), true);
          for (const l of constraints.locks) yLocks.set(cellKey(l.name, l.day), true);
        });
      }
      syncSets();
    });
    const aw = provider.awareness;
    aw.setLocalState({ user: { name: userName, color: colorFor(userName) } });
    const syncPeers = () =>
      setPeers([...aw.getStates().values()].map((s) => (s as { user?: { name?: string } }).user?.name ?? "?"));
    aw.on("change", syncPeers);
    syncPeers();

    return () => {
      manager.destroy();
      provider.destroy();
      ydoc.destroy();
      doc.current = null;
      undo.current = null;
    };
    // The doc lives exactly as long as the schedule id it was opened for.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scheduleId]);

  const edit = useCallback(
    (fn: (d: Y.Doc) => void) => {
      if (!canEdit || !doc.current) return;
      doc.current.transact(() => fn(doc.current!), ORIGIN);
    },
    [canEdit],
  );

  const boatOf = (name: string, day: string) =>
    rowsRef.current.find((r) => r.day === day && r.name === name)?.boat;

  const removeFromBoat = (d: Y.Doc, name: string, day: string) => {
    const boat = boatOf(name, day);
    if (boat === undefined) return;
    const arr = d.getMap<Y.Array<string>>("schedule").get(`${day}:${boat}`);
    const i = arr ? arr.toArray().indexOf(name) : -1;
    if (arr && i >= 0) arr.delete(i, 1);
  };

  const move = useCallback(
    (name: string, day: string, toBoat: number) =>
      edit((d) => {
        if (boatOf(name, day) === toBoat) return;
        removeFromBoat(d, name, day);
        const map = d.getMap<Y.Array<string>>("schedule");
        let arr = map.get(`${day}:${toBoat}`);
        if (!arr) {
          arr = new Y.Array<string>();
          map.set(`${day}:${toBoat}`, arr);
        }
        arr.push([name]);
      }),
    [edit],
  );

  const swap = useCallback(
    (day: string, a: string, b: string) =>
      edit((d) => {
        const ba = boatOf(a, day);
        const bb = boatOf(b, day);
        if (ba === undefined || bb === undefined || ba === bb) return;
        const map = d.getMap<Y.Array<string>>("schedule");
        const arrA = map.get(`${day}:${ba}`);
        const arrB = map.get(`${day}:${bb}`);
        if (!arrA || !arrB) return;
        const ia = arrA.toArray().indexOf(a);
        const ib = arrB.toArray().indexOf(b);
        if (ia < 0 || ib < 0) return;
        arrA.delete(ia, 1);
        arrA.insert(ia, [b]);
        arrB.delete(ib, 1);
        arrB.insert(ib, [a]);
      }),
    [edit],
  );

  const toggleLock = useCallback(
    (name: string, day: string) =>
      edit((d) => {
        const m = d.getMap<boolean>("locks");
        const k = cellKey(name, day);
        if (m.has(k)) m.delete(k);
        else m.set(k, true);
      }),
    [edit],
  );

  const toggleAbsent = useCallback(
    (name: string, day: string) =>
      edit((d) => {
        const m = d.getMap<boolean>("absences");
        const k = cellKey(name, day);
        if (m.has(k)) {
          m.delete(k);
        } else {
          m.set(k, true);
          removeFromBoat(d, name, day);
          d.getMap<boolean>("locks").delete(k);
        }
      }),
    [edit],
  );

  return {
    rows,
    locks,
    absences,
    connected,
    peers,
    levelOf,
    canonical,
    canUndo: canEdit && history.canUndo,
    canRedo: canEdit && history.canRedo,
    undo: () => undo.current?.undo(),
    redo: () => undo.current?.redo(),
    move,
    swap,
    toggleLock,
    toggleAbsent,
  };
}
