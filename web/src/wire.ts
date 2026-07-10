export const PROTOCOL_VERSION = 1;

export type Display =
  | { Text: string }
  | { Diff: { path: string; before: string; after: string } }
  | { Terminal: { command: string; stdout: string; stderr: string; exit_code: number } }
  | { Markdown: { text: string; title?: string; id?: string } }
  | { Code: { lang: string; filename?: string; text: string; title?: string; id?: string } }
  | { Html: { html: string; title?: string; id?: string } }
  | { Mermaid: { source: string; title?: string; id?: string } }
  | { Table: { columns: string[]; rows: string[][]; title?: string; id?: string } }
  | { Image: { mime: string; data: string; title?: string; id?: string } }
  | { Url: { url: string; title?: string; id?: string } };

export interface RuntimeSettings {
  backend: string;
  base_url: string;
  model: string;
  protocol: string;
  command_allowlist: string[];
  command_denylist: string[];
  temperature: number;
  max_tokens: number;
  max_turns: number;
  context_limit: number;
  top_p: number | null;
  top_k: number | null;
  min_p: number | null;
  presence_penalty: number | null;
  repeat_penalty: number | null;
  enable_thinking: boolean;
  preserve_thinking: boolean;
  memory: boolean;
  skills_dirs: string[];
  active_skills: string[];
  trace: boolean;
  trace_dir: string | null;
  trace_max_mb: number;
  system_prompt_override: string | null;
}

export interface SessionStats {
  turns: number; prompt_tokens: number; completion_tokens: number;
  reasoning_tokens: number; cached_tokens: number; cost_usd: number;
  tool_calls: number; tools_ok: number; tools_denied: number; tools_error: number;
  tools_timeout: number; tools_panic: number; tool_time_ms: number;
  turn_time_ms: number; context_events: number; errors: number;
  // optional: old servers omit these sub-agent subset counters.
  subagent_tool_calls?: number; subagent_turns?: number;
}

export interface DiscoveredSkill { name: string; description: string }

export interface ApprovalOrigin {
  delegation_id: string;
  subagent: string;
  depth: number;
}

export type WireEvent =
  | { type: "token"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "usage"; prompt_tokens: number; context_limit: number; turn: number; max_turns: number }
  | { type: "server_usage"; prompt_tokens: number; completion_tokens: number; turn: number;
      reasoning_tokens?: number; cached_tokens?: number; cost_usd?: number; turn_duration_ms?: number;
      parent_id?: string }
  | { type: "tool_start"; id: string; name: string; args: unknown; parent_id?: string }
  | { type: "tool_result"; id: string; name: string; status: string; duration_ms: number;
      content: string; display?: Display; parent_id?: string }
  | { type: "context"; kind: string; detail: Record<string, unknown> }
  | { type: "session_stats"; stats: SessionStats }
  | { type: "error"; message: string }
  | { type: "done"; reason: string }
  | { type: "sandbox_degraded"; mechanism: string; reason: string }
  | { type: "stream_retry"; discarded_text_chars: number; discarded_reasoning_chars: number }
  | { type: "subagent_start"; id: string; subagent_type: string; role?: string }
  | { type: "subagent_text"; id: string; text: string }
  | { type: "subagent_reasoning"; id: string; text: string }
  | { type: "subagent_stream_retry"; id: string;
      discarded_text_chars: number; discarded_reasoning_chars: number }
  | { type: "subagent_end"; id: string; outcome: string; stop?: string; detail?: string;
      turns: number; tool_calls: number; duration_ms: number };

export type Inbound =
  | { v: number; session_id: string; kind: "event"; payload: WireEvent }
  | { v: number; session_id: string; id: string; kind: "approval_request"; summary: string; command?: string; display?: Display; origin?: ApprovalOrigin }
  | { v: number; session_id: string; kind: "presence"; online: boolean }
  | { v: number; session_id: string; kind: "settings_state"; settings: RuntimeSettings; workspace: string; api_key_set: boolean; hard_floor: string[]; discovered_skills: DiscoveredSkill[]; sandbox_degraded?: { mechanism: string; reason: string } | null }
  | { v: number; session_id: string; kind: "settings_error"; message: string };

export type Decision = "approve" | "approve_always" | "deny";

export type Outbound =
  | { kind: "user_input"; text: string }
  | { kind: "approval_response"; id: string; decision: Decision }
  | { kind: "settings_get" }
  | { kind: "settings_update"; settings: RuntimeSettings };

/** Parse a raw WS text frame into an Inbound, or null if malformed/unknown. */
export function parseInbound(raw: string): Inbound | null {
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!v || typeof v !== "object") return null;
  const o = v as Record<string, unknown>;
  if (
    o.kind === "event" || o.kind === "approval_request" || o.kind === "presence" ||
    o.kind === "settings_state" || o.kind === "settings_error"
  ) {
    return o as unknown as Inbound;
  }
  return null;
}
