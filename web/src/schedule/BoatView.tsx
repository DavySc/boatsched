import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Row } from "../types";
import { useSession } from "../session";
import { DAY_NL_SHORT, dayName, levelLabel, sortDays } from "../i18n";
import { cellKey } from "./conflicts";

export interface BoatActions {
  move: (name: string, day: string, toBoat: number) => void;
  swap: (day: string, a: string, b: string) => void;
  toggleLock: (name: string, day: string) => void;
  toggleAbsent: (name: string, day: string) => void;
}

interface Props extends BoatActions {
  rows: Row[];
  roster: Row[];
  locks: Set<string>;
  absences: Set<string>;
  conflictCells: Set<string>;
  badBoats: Set<string>;
  canEdit: boolean;
  onOpenCard: (name: string, level: string) => void;
}

interface MenuState {
  name: string;
  day: string;
  x: number;
  y: number;
  mode: "main" | "swap";
}

// On a phone, open on today's camp day (or the first) instead of a 6-day scroll.
const WEEKDAY = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const initialDay = (days: string[]) => {
  if (!matchMedia("(max-width: 700px)").matches) return "all";
  const today = WEEKDAY[new Date().getDay()];
  return days.includes(today) ? today : (days[0] ?? "all");
};

interface DragState {
  name: string;
  day: string;
  x: number;
  y: number;
  over: string | null;
}

export function BoatView(p: Props) {
  const { cfg, boats, me, renameBoat } = useSession();
  const levelRank = Object.fromEntries(cfg.levels.map((l, i) => [l, i]));
  const restricted = new Set(cfg.restricted);
  const diverse = new Set(cfg.diverse);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [drag, setDrag] = useState<DragState | null>(null);
  const [renaming, setRenaming] = useState<number | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuTop, setMenuTop] = useState<number | null>(null);

  const boatLabel = (b: number) => boats[b] || `Boot ${b}`;
  const days = sortDays([...new Set(p.rows.map((r) => r.day))]);
  const [shown, setShown] = useState(() => initialDay(days));

  const rankOf = (l: string) => levelRank[l] ?? 99;
  const sortCrew = (crew: Row[]) =>
    [...crew].sort((a, b) => rankOf(a.level) - rankOf(b.level) || a.name.localeCompare(b.name));

  // ---- pointer drag (works for mouse, pen and touch via the handle) ----
  const startDrag = (e: React.PointerEvent, name: string, day: string) => {
    if (!p.canEdit) return;
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const d = { name, day, x: e.clientX, y: e.clientY, over: null };
    dragRef.current = d;
    setDrag(d);
  };
  const overTarget = (x: number, y: number, day: string): string | null => {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-kid],[data-boat-card]");
    if (!el || el.dataset.day !== day) return null;
    return el.dataset.kid ? `kid:${el.dataset.kid}` : `boat:${el.dataset.boat}`;
  };
  const moveDrag = (e: React.PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const next = { ...d, x: e.clientX, y: e.clientY, over: overTarget(e.clientX, e.clientY, d.day) };
    dragRef.current = next;
    setDrag(next);
  };
  const endDrag = (e: React.PointerEvent) => {
    const d = dragRef.current;
    dragRef.current = null;
    setDrag(null);
    if (!d) return;
    const target = overTarget(e.clientX, e.clientY, d.day);
    if (!target) return;
    const [kind, value] = [target.slice(0, target.indexOf(":")), target.slice(target.indexOf(":") + 1)];
    if (kind === "kid" && value !== d.name) p.swap(d.day, d.name, value);
    else if (kind === "boat") p.move(d.name, d.day, Number(value));
  };

  // Keep the menu on screen: shift it up when it would run past the bottom edge.
  useLayoutEffect(() => {
    if (!menu || !menuRef.current) return setMenuTop(null);
    const h = menuRef.current.offsetHeight;
    setMenuTop(Math.max(8, Math.min(menu.y, window.innerHeight - h - 8)));
  }, [menu]);

  useEffect(() => {
    if (!menu) return;
    const close = (e: Event) => {
      if (!(e.target as HTMLElement).closest(".menu")) setMenu(null);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setMenu(null);
    document.addEventListener("pointerdown", close);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", close);
      document.removeEventListener("keydown", esc);
    };
  }, [menu]);

  const levelOfName = (name: string) => p.roster.find((r) => r.name === name)?.level ?? "";

  const crewItem = (name: string, level: string, day: string, boat: number | null) => {
    const k = cellKey(name, day);
    const locked = p.locks.has(k);
    const absent = p.absences.has(k);
    return (
      <li
        key={name}
        data-kid={name}
        data-day={day}
        className={[
          locked ? "locked" : "",
          p.conflictCells.has(k) ? "conflict" : "",
          drag?.over === `kid:${name}` ? "drop-target" : "",
          drag?.name === name && drag.day === day ? "dragging" : "",
        ].join(" ")}
      >
        {p.canEdit && !absent && (
          <button
            className="handle"
            aria-label={`Sleep ${name}`}
            onPointerDown={(e) => startDrag(e, name, day)}
            onPointerMove={moveDrag}
            onPointerUp={endDrag}
            onPointerCancel={() => {
              dragRef.current = null;
              setDrag(null);
            }}
          >
            ⠿
          </button>
        )}
        <span className={`lv lv-${level}`} title={levelLabel(level)}>
          {level}
        </span>
        <button className="name-btn" onClick={() => p.onOpenCard(name, level)}>
          {name}
        </button>
        {locked && <span title="Vergrendeld: blijft staan bij opnieuw indelen">🔒</span>}
        {p.canEdit && (
          <button
            className="icon-btn"
            aria-label={`Acties voor ${name}`}
            aria-haspopup="menu"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              setMenu({ name, day, x: Math.min(r.left, window.innerWidth - 230), y: r.bottom + 4, mode: "main" });
            }}
          >
            ⋮
          </button>
        )}
        {boat === null && absent && <span className="badge muted">afwezig</span>}
      </li>
    );
  };

  const section = (day: string) => {
    const dayRows = p.rows.filter((r) => r.day === day);
    const boatIds = Array.from({ length: cfg.boats }, (_, i) => i + 1);
    for (const r of dayRows) if (!boatIds.includes(r.boat)) boatIds.push(r.boat);
    const entries = boatIds.map((b) => ({ b, crew: sortCrew(dayRows.filter((r) => r.boat === b)) }));
    entries.sort((x, y) => {
      if (!x.crew.length || !y.crew.length) return x.crew.length ? -1 : y.crew.length ? 1 : x.b - y.b;
      const kx = [rankOf(x.crew[0].level), rankOf(x.crew[x.crew.length - 1].level)];
      const ky = [rankOf(y.crew[0].level), rankOf(y.crew[y.crew.length - 1].level)];
      return kx[0] - ky[0] || kx[1] - ky[1] || x.b - y.b;
    });
    const placed = new Set(dayRows.map((r) => cellKey(r.name, day)));
    const unplaced = p.roster.filter((r) => !placed.has(cellKey(r.name, day)) && !p.absences.has(cellKey(r.name, day)));
    const absentHere = p.roster.filter((r) => p.absences.has(cellKey(r.name, day)));
    const mixedCount = entries.filter((e) => new Set(e.crew.map((c) => c.level)).size > 1).length;
    return (
      <section className={`day${shown === "all" || shown === day ? "" : " off"}`} key={day}>
        <div className="day-head">
          <h3>{dayName(day)}</h3>
          {restricted.has(day) && <span className="badge restricted">gelijk / aangrenzend niveau</span>}
          {diverse.has(day) && <span className="badge diverse">gemengd niveau</span>}
          <span className="meta">
            {restricted.has(day) ? `${mixedCount} gemengde ${mixedCount === 1 ? "boot" : "boten"}` : `${entries.filter((e) => e.crew.length).length} boten`}
          </span>
        </div>
        <div className="grid">
          {entries.map(({ b, crew }) => {
            const levels = [...new Set(crew.map((c) => c.level))];
            const mixed = levels.length > 1;
            return (
              <div
                key={b}
                className={["boat", mixed ? "mixed" : "", p.badBoats.has(`${day}|${b}`) ? "bad" : "", drag?.over === `boat:${b}` && drag.day === day ? "over" : ""].join(" ")}
                data-boat-card
                data-day={day}
                data-boat={b}
              >
                <div className="boat-top">
                  {renaming === b ? (
                    <input
                      className="boat-name-input"
                      autoFocus
                      defaultValue={boats[b] ?? ""}
                      placeholder={`Boot ${b}`}
                      aria-label={`Naam van boot ${b}`}
                      onBlur={(e) => {
                        setRenaming(null);
                        const name = e.target.value.trim() || null;
                        if (name !== (boats[b] ?? null)) void renameBoat(b, name).catch(() => {});
                      }}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") e.currentTarget.blur();
                        if (e.key === "Escape") setRenaming(null);
                      }}
                    />
                  ) : (
                    <button className="boat-num" disabled={!me.is_admin} onClick={() => setRenaming(b)} title={me.is_admin ? "Naam wijzigen" : undefined}>
                      {boatLabel(b)}
                    </button>
                  )}
                  <span className={`boat-tag${levels.length === 1 ? ` lv-${levels[0]}` : ""}`}>
                    {crew.length === 0 ? "leeg" : mixed ? `${levels.length} niveaus` : levels[0]} · {crew.length}
                  </span>
                </div>
                <ul className="crew">
                  {crew.map((c) => crewItem(c.name, c.level, day, b))}
                  {crew.length === 0 && <li className="empty-note">Sleep hier iemand naartoe</li>}
                </ul>
              </div>
            );
          })}
          {unplaced.length > 0 && (
            <div className="boat bad">
              <div className="boat-top">
                <span className="boat-num">Niet ingedeeld</span>
                <span className="boat-tag">{unplaced.length}</span>
              </div>
              <ul className="crew">{unplaced.map((r) => crewItem(r.name, r.level, day, 0))}</ul>
            </div>
          )}
          {absentHere.length > 0 && (
            <div className="boat">
              <div className="boat-top">
                <span className="boat-num">Afwezig</span>
                <span className="boat-tag">{absentHere.length}</span>
              </div>
              <ul className="crew">{absentHere.map((r) => crewItem(r.name, r.level, day, null))}</ul>
            </div>
          )}
        </div>
      </section>
    );
  };

  const menuEl = () => {
    if (!menu) return null;
    const { name, day } = menu;
    const k = cellKey(name, day);
    const current = p.rows.find((r) => r.day === day && r.name === name)?.boat;
    const others = p.rows.filter((r) => r.day === day && r.name !== name && r.boat !== current);
    const run = (fn: () => void) => () => {
      fn();
      setMenu(null);
    };
    return (
      <div className="menu" role="menu" ref={menuRef} style={{ left: menu.x, top: menuTop ?? menu.y }}>
        {menu.mode === "main" ? (
          <>
            <h4>
              {name} · {dayName(day)}
            </h4>
            <h4>Verplaats naar</h4>
            <div className="menu-boats">
              {Array.from({ length: cfg.boats }, (_, i) => i + 1)
                .filter((b) => b !== current)
                .map((b) => (
                  <button key={b} role="menuitem" aria-label={`Verplaats naar ${boatLabel(b)}`} onClick={run(() => p.move(name, day, b))}>
                    {boatLabel(b)}
                  </button>
                ))}
            </div>
            <button role="menuitem" onClick={() => setMenu({ ...menu, mode: "swap" })}>
              Wissel met…
            </button>
            <button role="menuitem" onClick={run(() => p.toggleLock(name, day))}>
              {p.locks.has(k) ? "Ontgrendel" : "Vergrendel (blijft bij opnieuw indelen)"}
            </button>
            <button role="menuitem" onClick={run(() => p.toggleAbsent(name, day))}>
              {p.absences.has(k) ? "Niet meer afwezig" : `Afwezig op ${dayName(day)}`}
            </button>
          </>
        ) : (
          <>
            <h4>Wissel {name} met</h4>
            {sortCrew(others).map((r) => (
              <button key={r.name} role="menuitem" onClick={run(() => p.swap(day, name, r.name))}>
                {r.name} · {boatLabel(r.boat)} ({levelOfName(r.name) || r.level})
              </button>
            ))}
            {others.length === 0 && <h4>Niemand om mee te wisselen</h4>}
          </>
        )}
      </div>
    );
  };

  return (
    <div>
      {days.length > 1 && (
        <div className="day-tabs seg" role="group" aria-label="Dag">
          <button aria-pressed={shown === "all"} onClick={() => setShown("all")}>Alle</button>
          {days.map((d) => (
            <button key={d} aria-pressed={shown === d} aria-label={dayName(d)} onClick={() => setShown(d)}>
              {DAY_NL_SHORT[d] ?? d}
            </button>
          ))}
        </div>
      )}
      {days.map(section)}
      {menuEl()}
      {drag && (
        <div className="drag-ghost" style={{ left: drag.x, top: drag.y }}>
          {drag.name}
        </div>
      )}
    </div>
  );
}
