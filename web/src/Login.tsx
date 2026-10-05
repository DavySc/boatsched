import { useState } from "react";
import { api, errMsg } from "./api";

export function Login({ onDone }: { onDone: () => void }) {
  const [name, setName] = useState("");
  const [pw, setPw] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr("");
    if (!name.trim()) return setErr("Vul uw naam in.");
    setBusy(true);
    try {
      await api("/api/auth/login", { json: { name: name.trim(), password: pw } });
      onDone();
    } catch (e) {
      setErr(errMsg(e) || "Inloggen mislukt.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-overlay">
      <form className="login-card" onSubmit={(e) => void submit(e)}>
        <div className="kicker" style={{ textAlign: "center", marginBottom: 12 }}>Zeilkamp PZK</div>
        <h1>Boot Rooster</h1>
        <p className="login-sub">Instructeur inloggen</p>
        <label className="login-label" htmlFor="login-name">Naam</label>
        <input id="login-name" className="pwd-input" placeholder="Uw naam" autoComplete="username" autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        <label className="login-label" htmlFor="login-pw">Wachtwoord</label>
        <input id="login-pw" type="password" className="pwd-input" placeholder="Wachtwoord" autoComplete="current-password" value={pw} onChange={(e) => setPw(e.target.value)} />
        {err && <p className="err" role="alert">{err}</p>}
        <button type="submit" className="btn-sm primary" style={{ width: "100%", marginTop: 18, padding: 14, fontSize: 15 }} disabled={busy}>
          Inloggen
        </button>
      </form>
    </div>
  );
}
