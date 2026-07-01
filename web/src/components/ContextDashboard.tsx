import { useState } from "react";
import type { RuntimeSettings, SessionStats } from "../wire";
import { loadDashExpanded, saveDashExpanded } from "../storage";
import { StatsPanel } from "./StatsPanel";

function fmt(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k` : `${n}`;
}

export function ContextDashboard(
  { usage, settings, toolCount, artifactCount, stats }:
  { usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number;
    stats: SessionStats | null },
) {
  const [expanded, setExpanded] = useState(loadDashExpanded);
  const toggle = () => setExpanded((e) => { const next = !e; saveDashExpanded(next); return next; });

  const pct = usage ? Math.min(100, Math.round((usage.promptTokens / usage.contextLimit) * 100)) : 0;
  const over = pct >= 80;
  const fill = over ? "var(--state-error)" : "var(--accent)";

  return (
    <div style={{ background: "var(--surface-base)", borderTop: "1px solid var(--border)" }}>
      <button onClick={toggle} aria-label="context usage" aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 py-2 text-left">
        <span className="h-2 w-2 shrink-0 rounded-full"
          style={{ background: usage ? fill : "var(--text-muted)" }} />
        <span className="font-mono text-xs shrink-0" style={{ color: "var(--text-strong)" }}>
          {usage ? `${fmt(usage.promptTokens)} / ${fmt(usage.contextLimit)}` : "— / —"}
        </span>
        <span className="relative h-1.5 flex-1 overflow-hidden rounded-full"
          style={{ background: "var(--surface-overlay)" }}>
          <span className="absolute inset-y-0 left-0 rounded-full"
            style={{ width: `${pct}%`, background: fill }} />
        </span>
        <span className="font-mono text-xs shrink-0" style={{ color: "var(--text-muted)" }}>
          {usage ? `${pct}%` : ""}
        </span>
        <span className="shrink-0 text-xs" style={{ color: "var(--text-muted)" }}>{expanded ? "▾" : "▸"}</span>
      </button>

      {expanded && (
        <div className="space-y-1 px-3 pb-2 font-mono text-xs" style={{ color: "var(--text-muted)" }}>
          {settings && (
            <div>model {settings.model} · temp {settings.temperature}</div>
          )}
          {usage && (
            <div>turns {usage.turn}/{usage.maxTurns} · {toolCount} tools · {artifactCount} art</div>
          )}
          {settings && settings.active_skills.length > 0 && (
            <div>skills: {settings.active_skills.join(", ")}</div>
          )}
          <StatsPanel stats={stats} />
        </div>
      )}
    </div>
  );
}
