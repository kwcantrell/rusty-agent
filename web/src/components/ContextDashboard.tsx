import { useState } from "react";
import type { RuntimeSettings, SessionStats } from "../wire";
import { loadDashExpanded, saveDashExpanded } from "../storage";
import { StatsPanel } from "./StatsPanel";
import { blockMeter } from "./cliFormat";

function fmt(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k` : `${n}`;
}

// Claude Code-style status line under the prompt box:
//   12.4k / 196k ▂▂▂░░░░░░░ 6% · qwen3.6 · turn 3/40        ▸
// Clicking toggles the expanded detail (model/temp, counts, skills, stats).
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

  return (
    <div>
      <button onClick={toggle} aria-label="context usage" aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 pb-2 text-left"
        style={{ color: "var(--cli-dim)" }}>
        <span className="shrink-0">{usage ? `${fmt(usage.promptTokens)} / ${fmt(usage.contextLimit)}` : "— / —"}</span>
        <span aria-hidden className="shrink-0" style={{ color: over ? "var(--cli-err)" : "var(--cli-dim)" }}>
          {blockMeter(pct)}
        </span>
        <span className="shrink-0">{usage ? `${pct}%` : ""}</span>
        {settings && <span className="truncate">· {settings.model}</span>}
        {usage && <span className="shrink-0">· turn {usage.turn}/{usage.maxTurns}</span>}
        <span className="ml-auto shrink-0">{expanded ? "▾" : "▸"}</span>
      </button>

      {expanded && (
        <div className="space-y-1 px-3 pb-2" style={{ color: "var(--cli-dim)" }}>
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
