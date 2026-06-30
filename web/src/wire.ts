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
  | { Image: { mime: string; data: string; title?: string; id?: string } };

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
}

export interface DiscoveredSkill { name: string; description: string }

export type WireEvent =
  | { type: "token"; text: string }
  | { type: "reasoning"; text: string }
  | { type: "usage"; prompt_tokens: number; context_limit: number; turn: number; max_turns: number }
  | { type: "server_usage"; prompt_tokens: number; completion_tokens: number; turn: number }
  | { type: "tool_start"; name: string; args: unknown }
  | { type: "tool_result"; name: string; content: string; display?: Display }
  | { type: "error"; message: string }
  | { type: "done"; reason: string };

export type Inbound =
  | { v: number; session_id: string; kind: "event"; payload: WireEvent }
  | { v: number; session_id: string; id: string; kind: "approval_request"; summary: string; command?: string; display?: Display }
  | { v: number; session_id: string; kind: "presence"; online: boolean }
  | { v: number; session_id: string; kind: "settings_state"; settings: RuntimeSettings; workspace: string; api_key_set: boolean; hard_floor: string[]; discovered_skills: DiscoveredSkill[] }
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
