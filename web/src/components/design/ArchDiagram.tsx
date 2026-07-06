import type { ArchitectureSnapshot, BlockId } from "./architecture";

interface BlockDef { id: BlockId; label: string; sub: (s: ArchitectureSnapshot) => string;
  badge?: (s: ArchitectureSnapshot) => string | null }

const BLOCKS: BlockDef[] = [
  { id: "prompt", label: "Prompt", sub: (s) => `~${s.prompt.est_tokens} tok`,
    badge: (s) => (s.prompt.override_active ? "override" : null) },
  { id: "model", label: "Model", sub: (s) => `${s.model.model} (${s.model.backend})` },
  { id: "context", label: "Context", sub: (s) => `${Math.round(s.context.context_limit / 1024)}k window`,
    badge: (s) => (s.context.memory_enabled ? "memory on" : "memory off") },
  { id: "loop", label: "Agent Loop", sub: (s) => `${s.loop.max_turns} turns max`,
    badge: (s) => (s.loop.subagents_enabled ? "subagents" : null) },
  { id: "tools", label: "Tools", sub: (s) => s.tools.map((t) => t.name).slice(0, 3).join(", ") + "…",
    badge: (s) => `${s.tools.length} tools` },
  { id: "policy", label: "Policy", sub: (s) => `${s.policy.allowlist.length} allowed / ${s.policy.denylist.length} denied` },
  { id: "sandbox", label: "Sandbox", sub: (s) => s.sandbox.image ?? s.sandbox.mechanism,
    badge: (s) => (s.sandbox.degraded ? "degraded" : null) },
];

/** Row layout: [prompt model context] feed [loop]; [tools policy sandbox] execute below. */
const ROWS: BlockId[][] = [["prompt", "model", "context"], ["loop"], ["tools", "policy", "sandbox"]];

export function ArchDiagram({ snapshot, selected, onSelect }: {
  snapshot: ArchitectureSnapshot; selected: BlockId | null; onSelect: (b: BlockId) => void;
}) {
  const byId = Object.fromEntries(BLOCKS.map((b) => [b.id, b])) as Record<BlockId, BlockDef>;
  return (
    <div className="relative flex flex-col gap-2 p-3" data-testid="arch-diagram">
      {ROWS.map((row, ri) => (
        <div key={ri} className="flex justify-center gap-2">
          {ri > 0 && <ArrowRow />}
          {row.map((id) => {
            const b = byId[id];
            const on = selected === id;
            const badge = b.badge?.(snapshot);
            return (
              <button key={id} aria-pressed={on} onClick={() => onSelect(id)}
                className="min-w-0 flex-1 rounded-lg px-3 py-2 text-left"
                style={{ maxWidth: "14rem", border: `1px solid ${on ? "var(--accent)" : "var(--border)"}`,
                  background: on ? "var(--surface-raised)" : "var(--surface-overlay)" }}>
                <span className="block text-xs font-semibold" style={{ color: "var(--text-strong)" }}>
                  {b.label}
                </span>
                <span className="block truncate text-[11px]" style={{ color: "var(--text-muted)" }}>
                  {b.sub(snapshot)}
                </span>
                {badge && (
                  <span className="mt-1 inline-block rounded-full px-1.5 text-[10px]"
                    style={{ background: badge === "degraded" ? "var(--state-error)" : "var(--surface-base)",
                      color: badge === "degraded" ? "var(--accent-fg)" : "var(--text-muted)",
                      border: "1px solid var(--border)" }}>
                    {badge}
                  </span>
                )}
              </button>
            );
          })}
        </div>
      ))}
    </div>
  );
}

/** Downward arrow between rows (pure decoration). */
function ArrowRow() {
  return <span aria-hidden className="self-center text-sm" style={{ color: "var(--text-muted)" }}>↓</span>;
}
