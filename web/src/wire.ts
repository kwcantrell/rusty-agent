export const PROTOCOL_VERSION = 1;

export type Display =
  | { Text: string }
  | { Diff: { path: string; before: string; after: string } }
  | { Terminal: { command: string; stdout: string; stderr: string; exit_code: number } };

export type WireEvent =
  | { type: "token"; text: string }
  | { type: "tool_start"; name: string; args: unknown }
  | { type: "tool_result"; name: string; content: string; display?: Display }
  | { type: "error"; message: string }
  | { type: "done"; reason: string };

export type Inbound =
  | { v: number; session_id: string; kind: "event"; payload: WireEvent }
  | { v: number; session_id: string; id: string; kind: "approval_request"; summary: string; command?: string; display?: Display }
  | { v: number; session_id: string; kind: "presence"; online: boolean };

export type Decision = "approve" | "approve_always" | "deny";

export type Outbound =
  | { v: number; session_id: string; kind: "user_input"; text: string }
  | { v: number; session_id: string; id: string; kind: "approval_response"; decision: Decision };

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
  if (o.kind === "event" || o.kind === "approval_request" || o.kind === "presence") {
    return o as unknown as Inbound;
  }
  return null;
}
