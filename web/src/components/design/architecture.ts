export type BlockId = "model" | "loop" | "tools" | "policy" | "sandbox" | "context" | "prompt";

export interface ToolEntry {
  name: string; summary: string;
  kind: "builtin" | "mcp" | "memory" | "skills" | "context";
}

export interface ArchitectureSnapshot {
  model: { backend: string; base_url_host: string; model: string; protocol: string;
    temperature: number; top_p: number | null; top_k: number | null;
    enable_thinking: boolean; preserve_thinking: boolean };
  tools: ToolEntry[];
  policy: { allowlist: string[]; denylist: string[]; hard_floor: string[]; http_allow_hosts: string[] };
  sandbox: { mode: string; mechanism: string; image: string | null; network: boolean; degraded: string | null };
  context: { context_limit: number; max_tool_result_bytes: number; memory_enabled: boolean;
    recall_budget: number; compaction_model: string | null };
  loop: { max_turns: number; max_parallel_tools: number; subagents_enabled: boolean;
    subagent_max_depth: number; subagent_model: string | null };
  prompt: { est_tokens: number; override_active: boolean; override_chars: number | null };
}

/** One read-only IPC fetch; throws on invoke failure (pane shows retry). */
export async function fetchArchitecture(): Promise<ArchitectureSnapshot> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<ArchitectureSnapshot>("architecture_get");
}
