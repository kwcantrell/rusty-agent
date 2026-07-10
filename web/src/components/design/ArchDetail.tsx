import type { ArchitectureSnapshot, BlockId, ToolEntry } from "./architecture";

const dt = "text-[11px] uppercase tracking-wide";
const dd = "mb-2 text-sm";

function Row({ k, v }: { k: string; v: string }) {
  return (
    <div>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>{k}</dt>
      <dd className={dd} style={{ color: "var(--text-strong)" }}>{v}</dd>
    </div>
  );
}

function ToolsTable({ tools }: { tools: ToolEntry[] }) {
  return (
    <table className="w-full text-sm" style={{ color: "var(--text)" }}>
      <tbody>
        {tools.map((t) => (
          <tr key={t.name} style={{ borderBottom: "1px solid var(--border)" }}>
            <td className="py-1 pr-2 font-mono text-xs" style={{ color: "var(--text-strong)" }}>{t.name}</td>
            <td className="py-1 pr-2 text-xs">{t.summary}</td>
            <td className="py-1">
              <span className="rounded-full px-1.5 text-[10px]"
                style={{ background: "var(--surface-base)", color: "var(--text-muted)",
                  border: "1px solid var(--border)" }}>{t.kind}</span>
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function PolicyLists({ p }: { p: ArchitectureSnapshot["policy"] }) {
  return (
    <dl>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>Allowlist</dt>
      <dd className={dd}>{p.allowlist.length === 0 ? "—" : p.allowlist.join(", ")}</dd>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>Denylist (effective)</dt>
      <dd className={dd}>
        <ul>
          {p.denylist.map((d) => (
            <li key={d} className="flex items-center gap-2">
              <span className="font-mono text-xs">{d}</span>
              {p.hard_floor.includes(d) && (
                <span className="rounded-full px-1.5 text-[10px]"
                  style={{ background: "var(--surface-base)", color: "var(--state-error)",
                    border: "1px solid var(--state-error)" }}>hard floor</span>
              )}
            </li>
          ))}
        </ul>
      </dd>
      <dt className={dt} style={{ color: "var(--text-muted)" }}>HTTP allow hosts</dt>
      <dd className={dd}>{p.http_allow_hosts.length === 0 ? "— (all fetches need approval)" : p.http_allow_hosts.join(", ")}</dd>
    </dl>
  );
}

export function ArchDetail({ snapshot, block }: { snapshot: ArchitectureSnapshot; block: BlockId }) {
  const s = snapshot;
  return (
    <div className="min-h-0 overflow-y-auto p-3" data-testid="arch-detail">
      {block === "model" && (
        <dl>
          <Row k="Backend" v={`${s.model.backend} (${s.model.protocol})`} />
          <Row k="Endpoint" v={s.model.base_url_host} />
          <Row k="Model" v={s.model.model} />
          <Row k="Sampling" v={`temp ${s.model.temperature}, top_p ${s.model.top_p ?? "—"}, top_k ${s.model.top_k ?? "—"}`} />
          <Row k="Thinking" v={`${s.model.enable_thinking ? "on" : "off"}${s.model.preserve_thinking ? ", preserved in history" : ""}`} />
        </dl>
      )}
      {block === "tools" && <ToolsTable tools={s.tools} />}
      {block === "policy" && <PolicyLists p={s.policy} />}
      {block === "sandbox" && (
        <dl>
          <Row k="Mode" v={s.sandbox.mode} />
          <Row k="Mechanism" v={s.sandbox.mechanism} />
          <Row k="Image" v={s.sandbox.image ?? "—"} />
          <Row k="Network" v={s.sandbox.network ? "enabled" : "disabled"} />
          {s.sandbox.degraded && <Row k="Degraded" v={s.sandbox.degraded} />}
        </dl>
      )}
      {block === "context" && (
        <dl>
          <Row k="Context limit" v={`${s.context.context_limit.toLocaleString()} tokens`} />
          <Row k="Max tool result" v={`${s.context.max_tool_result_bytes.toLocaleString()} bytes`} />
          <Row k="Memory" v={s.context.memory_enabled ? `on (memory index budget ${s.context.memory_index_budget})` : "off"} />
          <Row k="Compaction model" v={s.context.compaction_model ?? "— (primary model)"} />
        </dl>
      )}
      {block === "loop" && (
        <dl>
          <Row k="Max turns" v={String(s.loop.max_turns)} />
          <Row k="Parallel tools" v={String(s.loop.max_parallel_tools)} />
          <Row k="Subagents" v={s.loop.subagents_enabled
            ? `on (depth ${s.loop.subagent_max_depth}${s.loop.subagent_model ? `, model ${s.loop.subagent_model}` : ""})`
            : "off"} />
          <Row k="Stream idle timeout" v={`${s.loop.stream_idle_timeout_secs}s`} />
        </dl>
      )}
      {block === "prompt" && (
        <dl>
          <Row k="Composed size" v={`~${s.prompt.est_tokens} tokens`} />
          <Row k="Base prompt" v={s.prompt.override_active
            ? `override active (${s.prompt.override_chars} chars)` : "built-in"} />
        </dl>
      )}
    </div>
  );
}
