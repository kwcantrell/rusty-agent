import type { Item } from "../state";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ToolCall({ item }: { item: ToolItem }) {
  const running = item.status === "running";
  return (
    <div className="my-1 inline-flex items-center gap-2 rounded-md px-2 py-1 font-mono text-xs"
      style={{ background: "var(--surface-raised)", border: "1px solid var(--border)", color: "var(--text)" }}>
      <span className="rounded-full px-1.5 text-[10px]"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>{item.name}</span>
      <span style={{ color: running ? "var(--state-run)" : "var(--state-done)" }}>{running ? "…" : "✓"}</span>
    </div>
  );
}
