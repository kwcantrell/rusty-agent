import { useState } from "react";

export function PairingScreen({ onPaired }: { onPaired: (s: { sessionId: string; token: string }) => void }) {
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const pair = async () => {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch("/pair", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ pairing_code: code.trim() }),
      });
      if (!r.ok) {
        const body = (await r.json().catch(() => ({}))) as { error?: string };
        setError(body.error ?? `pairing failed (${r.status})`);
        return;
      }
      const body = (await r.json()) as { session_id: string; session_token: string };
      onPaired({ sessionId: body.session_id, token: body.session_token });
    } catch {
      setError("could not reach the control plane");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex h-full flex-col items-center justify-center gap-3" style={{ color: "var(--text-strong)" }}>
      <h1 className="text-lg">Pair with your agent</h1>
      <input
        className="rounded-lg px-3 py-2 text-center font-mono tracking-widest outline-none"
        style={{ background: "var(--surface-overlay)", color: "var(--text-strong)", border: "1px solid var(--border)" }}
        value={code}
        onChange={(e) => setCode(e.target.value)}
        placeholder="pairing code"
        onKeyDown={(e) => { if (e.key === "Enter") pair(); }}
      />
      <button onClick={pair} disabled={busy || !code.trim()} className="rounded-lg px-4 py-2 hover:opacity-90 disabled:opacity-50"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
        {busy ? "Pairing…" : "Pair"}
      </button>
      {error && <div style={{ color: "var(--state-error)" }}>{error}</div>}
    </div>
  );
}
