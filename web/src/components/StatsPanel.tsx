import type { SessionStats } from "../wire";

const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));

export function StatsPanel({ stats }: { stats: SessionStats | null }) {
  if (!stats) return null;
  const failed = stats.tools_denied + stats.tools_error + stats.tools_timeout + stats.tools_panic;
  const rows: Array<[string, string]> = [
    ["Turns", String(stats.turns)],
    ["Tokens in / out", `${k(stats.prompt_tokens)} / ${k(stats.completion_tokens)}`],
    ["Reasoning / cached", `${k(stats.reasoning_tokens)} / ${k(stats.cached_tokens)}`],
    ["Tool calls", `${stats.tool_calls} (${failed} failed)`],
    ["Time in tools", `${(stats.tool_time_ms / 1000).toFixed(1)}s`],
    ["Model time", `${(stats.turn_time_ms / 1000).toFixed(1)}s`],
    ["Context events", String(stats.context_events)],
  ];
  if ((stats.subagent_tool_calls ?? 0) > 0 || (stats.subagent_turns ?? 0) > 0)
    rows.push(["Sub-agent", `${stats.subagent_tool_calls ?? 0} calls / ${stats.subagent_turns ?? 0} turns`]);
  if (stats.cost_usd > 0) rows.push(["Cost", `$${stats.cost_usd.toFixed(4)}`]);
  return (
    <section aria-label="Session stats" className="space-y-1 text-sm">
      <h3 className="font-medium">Session stats</h3>
      <dl className="grid grid-cols-2 gap-x-3 gap-y-0.5">
        {rows.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="text-right tabular-nums">{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
