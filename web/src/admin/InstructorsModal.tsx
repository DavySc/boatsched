import { useEffect, useState } from "react";
import { api, errMsg } from "../api";
import { useSession } from "../session";
import { Modal } from "../ui/Modal";
import { useToast } from "../ui/Toast";
import type { Instructor, Member } from "../types";

export function InstructorsModal({ onClose }: { onClose: () => void }) {
  const { me } = useSession();
  const toast = useToast();
  const [list, setList] = useState<Instructor[]>([]);
  const [members, setMembers] = useState<Member[]>([]);
  const [target, setTarget] = useState<string | null>(me.is_admin ? null : me.name);
  const [pw, setPw] = useState("");
  const [err, setErr] = useState("");
  const [newName, setNewName] = useState("");
  const [newPw, setNewPw] = useState("");
  const [addErr, setAddErr] = useState("");
  const [comboOpen, setComboOpen] = useState(false);
  const [active, setActive] = useState(-1);

  const reload = () => api<Instructor[]>("/instructors").then(setList).catch((e) => toast(errMsg(e), "error"));

  useEffect(() => {
    if (!me.is_admin) return;
    void reload();
    api<Member[]>("/api/members").then(setMembers).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const submitPw = async () => {
    if (!pw || !target) return;
    setErr("");
    try {
      await api("/set-password", { json: { name: target, password: pw } });
      setTarget(me.is_admin ? null : target);
      setPw("");
      toast("Wachtwoord opgeslagen");
      if (me.is_admin) void reload();
    } catch (e) {
      setErr(errMsg(e));
    }
  };

  const addInstructor = async () => {
    setAddErr("");
    if (!newName.trim() || !newPw) return setAddErr("Naam en wachtwoord zijn verplicht.");
    try {
      await api("/instructor", { json: { name: newName.trim(), password: newPw } });
      setNewName("");
      setNewPw("");
      void reload();
    } catch (e) {
      setAddErr(errMsg(e));
    }
  };

  const toggleAdmin = async (id: number, makeAdmin: boolean) => {
    try {
      await api(`/api/instructors/${id}/set-admin`, { json: { is_admin: makeAdmin } });
      void reload();
    } catch (e) {
      toast(errMsg(e), "error");
    }
  };

  const hits = newName
    ? members.filter((m) => m.name.toLowerCase().includes(newName.toLowerCase())).slice(0, 30)
    : [];

  return (
    <Modal title={me.is_admin ? "Instructeur beheer" : "Mijn wachtwoord"} onClose={onClose}>
      {me.is_admin && (
        <ul className="inst-list">
          {list.length === 0 && <li style={{ padding: "10px 0", color: "#8a7a62" }}>Geen instructeurs in database.</li>}
          {list.map((i) => (
            <li className="inst-row" key={i.id}>
              <span className="inst-name">
                {i.name}
                {i.is_admin && <span className="badge-admin">admin</span>}
              </span>
              <span className={`inst-status${i.has_password ? " set" : ""}`}>
                {i.has_password ? "Wachtwoord ingesteld" : "Geen wachtwoord"}
              </span>
              <button className="btn-sm" onClick={() => setTarget(i.name)}>
                {i.has_password ? "Wijzigen" : "Instellen"}
              </button>
              {i.id !== me.member_id && (
                <button className="btn-sm" onClick={() => void toggleAdmin(i.id, !i.is_admin)}>
                  {i.is_admin ? "Admin verwijderen" : "Admin maken"}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
      {target && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void submitPw();
          }}
        >
          {me.is_admin && <div className="modal-sep" />}
          <label className="form-label" htmlFor="pw-new">
            Wachtwoord instellen voor {target}
          </label>
          <input id="pw-new" type="password" className="pwd-input" placeholder="Nieuw wachtwoord" autoComplete="new-password" value={pw} onChange={(e) => setPw(e.target.value)} autoFocus />
          <div className="modal-actions">
            <button type="submit" className="btn-sm primary">
              Opslaan
            </button>
            {me.is_admin && (
              <button type="button" className="btn-sm" onClick={() => setTarget(null)}>
                Annuleren
              </button>
            )}
          </div>
          {err && <p className="err">{err}</p>}
        </form>
      )}
      {me.is_admin && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            void addInstructor();
          }}
        >
          <div className="modal-sep" />
          <label className="form-label" htmlFor="inst-name">
            Nieuwe instructeur toevoegen
          </label>
          <div className="combo-wrap">
            <input
              id="inst-name"
              className="pwd-input"
              placeholder="Zoek op naam…"
              autoComplete="off"
              role="combobox"
              aria-expanded={comboOpen && hits.length > 0}
              aria-controls="member-combo"
              value={newName}
              onChange={(e) => {
                setNewName(e.target.value);
                setComboOpen(true);
                setActive(-1);
              }}
              onFocus={() => setComboOpen(true)}
              onBlur={() => setTimeout(() => setComboOpen(false), 150)}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setActive((a) => Math.min(hits.length - 1, a + 1));
                } else if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setActive((a) => Math.max(0, a - 1));
                } else if (e.key === "Enter" && comboOpen && active >= 0 && hits[active]) {
                  e.preventDefault();
                  setNewName(hits[active].name);
                  setComboOpen(false);
                } else if (e.key === "Escape" && comboOpen) {
                  e.stopPropagation();
                  setComboOpen(false);
                }
              }}
            />
            {comboOpen && hits.length > 0 && (
              <ul className="combo-list" id="member-combo" role="listbox">
                {hits.map((m, i) => (
                  <li
                    key={m.id}
                    role="option"
                    aria-selected={i === active}
                    className={i === active ? "active" : ""}
                    onMouseDown={(e) => {
                      e.preventDefault();
                      setNewName(m.name);
                      setComboOpen(false);
                    }}
                  >
                    {m.name}
                  </li>
                ))}
              </ul>
            )}
          </div>
          <input type="password" className="pwd-input" placeholder="Wachtwoord" aria-label="Wachtwoord nieuwe instructeur" autoComplete="new-password" value={newPw} onChange={(e) => setNewPw(e.target.value)} />
          <button type="submit" className="btn-sm primary">
            Instructeur toevoegen
          </button>
          {addErr && <p className="err">{addErr}</p>}
        </form>
      )}
    </Modal>
  );
}
