import { useEffect, useState } from "react";
import { api, errMsg } from "../api";
import { levelLabel } from "../i18n";
import { useSession } from "../session";
import { Modal } from "../ui/Modal";
import { useToast } from "../ui/Toast";
import type { Member, Progression } from "../types";

const LEVEL_COLOR: Record<string, string> = { k: "var(--k)", b: "var(--b)", A: "var(--A)", B: "var(--B)" };

export function MembersModal({ onClose, seasonId }: { onClose: () => void; seasonId: number | null }) {
  const toast = useToast();
  const { cfg, me } = useSession();
  const [tab, setTab] = useState<"list" | "prog">("list");
  const [members, setMembers] = useState<Member[]>([]);
  const [editing, setEditing] = useState<number | null>(null);
  const [draft, setDraft] = useState("");
  const [editErr, setEditErr] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [importMsg, setImportMsg] = useState("");
  const [importErr, setImportErr] = useState("");
  const [prog, setProg] = useState<Progression | null>(null);
  const [levelState, setLevelState] = useState<Record<string, "ok" | "bad">>({});

  const reload = () => api<Member[]>("/api/members").then(setMembers).catch((e) => toast(errMsg(e), "error"));
  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (tab === "prog") api<Progression>("/api/members/progression").then(setProg).catch((e) => toast(errMsg(e), "error"));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  const rename = async (id: number) => {
    const name = draft.trim();
    if (!name) return setEditErr("Naam mag niet leeg zijn.");
    try {
      await api(`/api/members/${id}`, { method: "PUT", json: { name } });
      setEditing(null);
      void reload();
    } catch (e) {
      setEditErr(errMsg(e));
    }
  };

  const remove = async (m: Member) => {
    if (!confirm(`Lid "${m.name}" verwijderen?`)) return;
    try {
      await api(`/api/members/${m.id}`, { method: "DELETE" });
      void reload();
    } catch (e) {
      toast(errMsg(e), "error");
    }
  };

  const doImport = async () => {
    setImportMsg("");
    setImportErr("");
    if (!file) return setImportErr("Kies eerst een CSV bestand.");
    const form = new FormData();
    form.append("file", file);
    try {
      const d = await api<{ created: number; existing: number; total: number }>("/api/members/import", { form });
      setImportMsg(`✓ ${d.created} toegevoegd, ${d.existing} al aanwezig — totaal ${d.total} leden`);
      void reload();
    } catch (e) {
      setImportErr(`Import mislukt: ${errMsg(e)}`);
    }
  };

  const setLevel = async (memberId: number, sid: number, level: string) => {
    const key = `${memberId}:${sid}`;
    try {
      await api(`/enrollment/${sid}/member/${memberId}/level`, { method: "PATCH", json: { level } });
      setProg((p) =>
        p && { ...p, members: p.members.map((m) => (m.id === memberId ? { ...m, levels: { ...m.levels, [sid]: level } } : m)) },
      );
      setLevelState((s) => ({ ...s, [key]: "ok" }));
      setTimeout(() => setLevelState((s) => { const { [key]: _, ...rest } = s; return rest; }), 1200);
    } catch (e) {
      setLevelState((s) => ({ ...s, [key]: "bad" }));
      toast(`Opslaan mislukt: ${errMsg(e)}`, "error");
    }
  };

  return (
    <Modal title="Leden" onClose={onClose} wide>
      <div className="seg" style={{ marginBottom: 14 }}>
        <button aria-pressed={tab === "list"} onClick={() => setTab("list")}>Lijst</button>
        <button aria-pressed={tab === "prog"} onClick={() => setTab("prog")}>Voortgang</button>
      </div>
      {tab === "list" ? (
        <>
          <div style={{ fontSize: 13, marginBottom: 8 }}>{members.length} leden</div>
          <ul className="inst-list">
            {members.length === 0 && <li style={{ padding: "10px 0", color: "#8a7a62" }}>Geen leden in database.</li>}
            {members.map((m) =>
              editing === m.id ? (
                <li className="inst-row" key={m.id}>
                  <input className="text-input" style={{ flex: 1 }} value={draft} autoFocus aria-label="Nieuwe naam" onChange={(e) => setDraft(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") void rename(m.id); if (e.key === "Escape") { e.stopPropagation(); setEditing(null); } }} />
                  <button className="btn-sm primary" onClick={() => void rename(m.id)}>Opslaan</button>
                  <button className="btn-sm" onClick={() => setEditing(null)}>Annuleren</button>
                  {editErr && <span style={{ fontSize: 12, color: "var(--accent)" }}>{editErr}</span>}
                </li>
              ) : (
                <li className="inst-row" key={m.id}>
                  <button className="icon-btn" title="Naam bewerken" aria-label={`Naam van ${m.name} bewerken`} onClick={() => { setEditing(m.id); setDraft(m.name); setEditErr(""); }}>✏</button>
                  <span className="inst-name">{m.name}</span>
                  <button className="icon-btn delete" title="Verwijderen" aria-label={`${m.name} verwijderen`} onClick={() => void remove(m)}>🗑</button>
                </li>
              ),
            )}
          </ul>
          <div className="modal-sep" />
          <p style={{ fontSize: 14, color: "var(--ink2)", marginBottom: 10 }}>
            CSV met een <code>name</code> kolom importeren om leden toe te voegen.
          </p>
          <input type="file" accept=".csv,text/csv" aria-label="CSV bestand" style={{ marginBottom: 8, display: "block" }} onChange={(e) => setFile(e.target.files?.[0] ?? null)} />
          <div className="modal-actions">
            <button className="btn-sm primary" onClick={() => void doImport()}>Importeren</button>
            {seasonId != null && (
              <a className="btn-sm" href={`/enrollment/${seasonId}/students/export`} download>
                Exporteer niveaus (CSV)
              </a>
            )}
          </div>
          {importMsg && <p className="ok-note">{importMsg}</p>}
          {importErr && <p className="err">{importErr}</p>}
        </>
      ) : !prog ? (
        <p>Laden…</p>
      ) : prog.seasons.length === 0 ? (
        <p>Nog geen seizoenen aangemaakt.</p>
      ) : prog.members.length === 0 ? (
        <p>Geen leden zonder instructeursrol.</p>
      ) : (
        <div className="prog-wrap">
          <table className="prog-table">
            <thead>
              <tr>
                <th>Naam</th>
                {prog.seasons.map((s) => <th key={s.id}>{s.year}</th>)}
              </tr>
            </thead>
            <tbody>
              {prog.members.map((m) => (
                <tr key={m.id}>
                  <td>{m.name}</td>
                  {prog.seasons.map((s) => {
                    const lv = m.levels[String(s.id)];
                    if (!lv) return <td key={s.id}>—</td>;
                    if (!me.is_admin)
                      return (
                        <td key={s.id}>
                          <span className="lv-pill" style={{ background: LEVEL_COLOR[lv] }}>{lv}</span>
                        </td>
                      );
                    const st = levelState[`${m.id}:${s.id}`];
                    return (
                      <td key={s.id}>
                        <select className={`level-edit ${st ?? ""}`} style={{ background: LEVEL_COLOR[lv] }} aria-label={`Niveau van ${m.name} in ${s.year}`} value={lv} onChange={(e) => void setLevel(m.id, s.id, e.target.value)}>
                          {cfg.levels.map((l) => <option key={l} value={l}>{l} · {levelLabel(l)}</option>)}
                        </select>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Modal>
  );
}
