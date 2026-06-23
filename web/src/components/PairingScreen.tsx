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
    <div className="flex h-full flex-col items-center justify-center gap-3 text-zinc-100">
      <h1 className="text-lg">Pair with your agent</h1>
      <input
        className="rounded bg-zinc-900 px-3 py-2 text-center font-mono tracking-widest outline-none"
        value={code}
        onChange={(e) => setCode(e.target.value)}
        placeholder="pairing code"
        onKeyDown={(e) => { if (e.key === "Enter") pair(); }}
      />
      <button onClick={pair} disabled={busy || !code.trim()} className="rounded bg-zinc-700 px-4 py-2 hover:bg-zinc-600 disabled:opacity-50">
        {busy ? "Pairing…" : "Pair"}
      </button>
      {error && <div className="text-red-400">{error}</div>}
    </div>
  );
}
