import { useEffect, useState } from "react";

const VERBS = ["Thinking", "Wrangling", "Percolating", "Noodling", "Brewing", "Riffing"];

/** Deterministic per-turn verb so a turn keeps one verb for its lifetime. */
export function busyVerb(turn: number): string {
  return VERBS[Math.abs(turn) % VERBS.length];
}

// Claude Code-style working indicator: `✳ Verb… (Ns)`. The seconds counter
// starts when the line mounts (i.e. when the turn starts). No interrupt hint:
// the runtime has no cancel path.
export function BusyLine({ turn }: { turn: number }) {
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setSecs((s) => s + 1), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <div className="my-2 flex gap-2 px-4" style={{ color: "var(--cli-dim)" }}>
      <span className="animate-pulse" style={{ color: "var(--cli-accent)" }}>✳</span>
      <span>{busyVerb(turn)}… ({secs}s)</span>
    </div>
  );
}
