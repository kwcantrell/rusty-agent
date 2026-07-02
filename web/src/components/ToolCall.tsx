import type { Item } from "../state";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ToolCall({ item }: { item: ToolItem }) {
  const running = item.status === "running";
  const failed = !!item.resultStatus && item.resultStatus !== "ok";
  const nested = !!item.parentId;
  const displayName = nested && item.name.startsWith("sub:") ? item.name.slice(4) : item.name;
  return (
    <div className="my-1 inline-flex items-center gap-2 rounded-md px-2 py-1 font-mono text-xs"
      style={{ background: "var(--surface-raised)", border: "1px solid var(--border)", color: "var(--text)",
        marginLeft: nested ? "1.25rem" : undefined }}>
      {nested && <span style={{ color: "var(--text-muted)" }}>↳</span>}
      <span className="rounded-full px-1.5 text-[10px]"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>{displayName}</span>
      <span style={{ color: running ? "var(--state-run)" : "var(--state-done)" }}>{running ? "…" : "✓"}</span>
      {failed && (
        <span className="rounded-full px-1.5 text-[10px]"
          style={{ border: "1px solid var(--state-error)", color: "var(--state-error)" }}>
          {item.resultStatus} · {item.durationMs}ms
        </span>
      )}
    </div>
  );
}
