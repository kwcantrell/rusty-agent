import type { Display, Inbound, RuntimeSettings } from "./wire";

export type ConnectionStatus = "connecting" | "open" | "closed" | "error";

export type Item =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; done?: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done"; content?: string; display?: Display }
  | { kind: "error"; message: string };

export interface PendingApproval {
  id: string;
  summary: string;
  command?: string;
  display?: Display;
}

export interface ConversationState {
  items: Item[];
  pendingApproval: PendingApproval | null;
  online: boolean;
  status: ConnectionStatus;
  // replay scaffolding (not rendered):
  userMsgs: string[]; // stored user messages for this session; index = turn
  turnIndex: number; // turns started so far
  inTurn: boolean; // has the current turn's user item been emitted?
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[]; discoveredSkills: import("./wire").DiscoveredSkill[] } | null;
  settingsError: string | null;
}

export type Action =
  | { type: "reset"; userMsgs: string[] }
  | { type: "frame"; frame: Inbound }
  | { type: "user_send"; text: string }
  | { type: "approval_sent" }
  | { type: "status"; status: ConnectionStatus };

export function initialState(userMsgs: string[]): ConversationState {
  return { items: [], pendingApproval: null, online: false, status: "connecting", userMsgs, turnIndex: 0, inTurn: false,
    settings: null, settingsMeta: null, settingsError: null };
}

/** Emit the stored user message that heads the current turn, if not already emitted. */
function startTurn(s: ConversationState): ConversationState {
  if (s.inTurn) return s;
  const text = s.userMsgs[s.turnIndex];
  const items = text !== undefined ? [...s.items, { kind: "user", text } as Item] : s.items;
  return { ...s, items, inTurn: true };
}

export function reduce(state: ConversationState, action: Action): ConversationState {
  switch (action.type) {
    case "reset":
      return initialState(action.userMsgs);
    case "status":
      return { ...state, status: action.status };
    case "user_send": {
      // Live send: the user item is emitted now, so the upcoming turn must not re-emit it.
      return { ...state, items: [...state.items, { kind: "user", text: action.text }], inTurn: true };
    }
    case "approval_sent":
      return { ...state, pendingApproval: null };
    case "frame":
      return reduceFrame(state, action.frame);
  }
}

function reduceFrame(state: ConversationState, frame: Inbound): ConversationState {
  if (frame.kind === "presence") return { ...state, online: frame.online };
  if (frame.kind === "settings_state") {
    return { ...state, settings: frame.settings,
      settingsMeta: { workspace: frame.workspace, apiKeySet: frame.api_key_set,
        hardFloor: frame.hard_floor, discoveredSkills: frame.discovered_skills },
      settingsError: null };
  }
  if (frame.kind === "settings_error") {
    return { ...state, settingsError: frame.message };
  }
  if (frame.kind === "approval_request") {
    return { ...state, pendingApproval: { id: frame.id, summary: frame.summary, command: frame.command, display: frame.display } };
  }
  // frame.kind === "event"
  const s = startTurn(state);
  const p = frame.payload;
  switch (p.type) {
    case "token": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "assistant" && last.done === undefined) {
        items[items.length - 1] = { ...last, text: last.text + p.text };
      } else {
        items.push({ kind: "assistant", text: p.text });
      }
      return { ...s, items };
    }
    case "reasoning": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "reasoning") {
        items[items.length - 1] = { ...last, text: last.text + p.text };
      } else {
        items.push({ kind: "reasoning", text: p.text });
      }
      return { ...s, items };
    }
    case "tool_start":
      return { ...s, items: [...s.items, { kind: "tool", name: p.name, args: p.args, status: "running" }] };
    case "tool_result": {
      const items = [...s.items];
      for (let i = items.length - 1; i >= 0; i--) {
        const it = items[i];
        if (it.kind === "tool" && it.status === "running" && it.name === p.name) {
          items[i] = { ...it, status: "done", content: p.content, display: p.display };
          break;
        }
      }
      return { ...s, items };
    }
    case "error":
      return { ...s, items: [...s.items, { kind: "error", message: p.message }] };
    case "done": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "assistant" && last.done === undefined) {
        items[items.length - 1] = { ...last, done: p.reason };
      }
      // Close the turn: next event starts a new one and re-emits the next user message.
      return { ...s, items, turnIndex: s.turnIndex + 1, inTurn: false };
    }
  }
}
