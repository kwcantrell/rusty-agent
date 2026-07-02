import { useMemo } from "react";
import type { Display, Inbound, RuntimeSettings, SessionStats } from "./wire";

export type ConnectionStatus = "connecting" | "open" | "closed" | "error";

export type Item =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; done?: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done";
      content?: string; display?: Display; resultStatus?: string; durationMs?: number }
  | { kind: "context"; text: string }
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
  usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
  serverUsage: { promptTokens: number; turn: number } | null;
  online: boolean;
  status: ConnectionStatus;
  // replay scaffolding (not rendered):
  userMsgs: string[]; // stored user messages for this session; index = turn
  turnIndex: number; // turns started so far
  inTurn: boolean; // has the current turn's user item been emitted?
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[]; discoveredSkills: import("./wire").DiscoveredSkill[] } | null;
  settingsError: string | null;
  sandboxDegraded: { mechanism: string; reason: string } | null;
  stats: SessionStats | null;
}

export type Action =
  | { type: "reset"; userMsgs: string[] }
  | { type: "frame"; frame: Inbound }
  | { type: "user_send"; text: string }
  | { type: "approval_sent" }
  | { type: "status"; status: ConnectionStatus }
  | { type: "dismiss_sandbox_banner" };

export function initialState(userMsgs: string[]): ConversationState {
  return { items: [], pendingApproval: null, usage: null, serverUsage: null, online: false, status: "connecting", userMsgs, turnIndex: 0, inTurn: false,
    settings: null, settingsMeta: null, settingsError: null, sandboxDegraded: null, stats: null };
}

/** One-line human summary of a context-management event, keyed by kind. */
function describeContext(kind: string, detail: Record<string, unknown>): string {
  switch (kind) {
    case "offloaded": return `offloaded ${detail.tool} result #${detail.id}`;
    case "compacted":
      return `compacted ${detail.turns_replaced} turns: ${detail.tokens_before} → ${detail.tokens_after} tokens`;
    case "compaction_failed": return `compaction failed: ${detail.reason}`;
    case "evicted": return `evicted ${detail.messages} messages (~${detail.est_tokens} tokens)`;
    default: return kind;
  }
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
    case "dismiss_sandbox_banner":
      return { ...state, sandboxDegraded: null };
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
      settingsError: null,
      sandboxDegraded: frame.sandbox_degraded ?? null };
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
    case "usage":
      return { ...s, usage: { promptTokens: p.prompt_tokens, contextLimit: p.context_limit, turn: p.turn, maxTurns: p.max_turns } };
    // The breakdown only needs the prompt total, so we intentionally keep only
    // promptTokens here and drop completion_tokens. Revisit if a chart needs it.
    case "server_usage":
      return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
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
          items[i] = { ...it, status: "done", content: p.content, display: p.display,
            resultStatus: p.status, durationMs: p.duration_ms };
          break;
        }
      }
      return { ...s, items };
    }
    case "context":
      return { ...s, items: [...s.items, { kind: "context", text: describeContext(p.kind, p.detail) }] };
    case "session_stats":
      return { ...s, stats: p.stats };
    case "sandbox_degraded":
      return { ...s, sandboxDegraded: { mechanism: p.mechanism, reason: p.reason } };
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
    // Forward compat: the backend may ship event types this UI doesn't know yet.
    // Without this, an unknown payload falls off the switch and reduces state to
    // undefined. Return the pre-startTurn state: an unrecognized event must not
    // open a turn or mutate anything.
    default:
      return state;
  }
}

/** Animation metadata derived from Item — never persisted. */
export type AnimatedItem = Item & {
  ts: number;
  streaming: boolean;
  progress: number;
};

export interface TurnGroup {
  items: AnimatedItem[];
  startTs: number;
  endTs: number;
  duration: number;
}

/**
 * Derives animation metadata from raw Item[] for consumption by animated components.
 * @param items - items from the reducer
 * @param now - current timestamp (for tests: fixed value)
 */
export function animatedItemsFrom(items: Item[], now: number): AnimatedItem[] {
  let ts = now;
  return items.map((item, i) => {
    const streaming = isStreamingItem(item, i === items.length - 1);
    const progress = streaming ? 0 : 1;
    const curTs = ts++;
    return { ...item, ts: curTs, streaming, progress } as AnimatedItem;
  });
}

// Tokens only ever extend the LAST item (the reducer appends to last-if-same-kind,
// else pushes a new item). So a reasoning/assistant block is live only while it is
// the trailing item; once any later block exists it can receive no more deltas and
// is finished. Tools carry their own explicit running/done status.
function isStreamingItem(item: Item, isLast: boolean): boolean {
  if (item.kind === "assistant") return isLast && item.done === undefined;
  if (item.kind === "reasoning") return isLast;
  if (item.kind === "tool" && item.status === "running") return true;
  return false;
}

/**
 * Groups animated items into turns, delimited by done signals.
 * Each turn starts with the first item after the previous turn's done (or the start).
 */
export function turnGroupsFrom(items: AnimatedItem[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  let currentGroup: AnimatedItem[] = [];

  for (const item of items) {
    currentGroup.push(item);
    if (item.kind === "assistant" && item.done !== undefined) {
      if (currentGroup.length > 0) {
        const startTs = currentGroup[0].ts;
        const endTs = currentGroup[currentGroup.length - 1].ts;
        groups.push({
          items: [...currentGroup],
          startTs,
          endTs,
          duration: endTs - startTs,
        });
      }
      currentGroup = [];
    }
  }

  // Flush any remaining items (e.g., if stream ended mid-turn)
  if (currentGroup.length > 0) {
    const startTs = currentGroup[0].ts;
    const endTs = currentGroup[currentGroup.length - 1].ts;
    groups.push({
      items: [...currentGroup],
      startTs,
      endTs,
      duration: endTs - startTs,
    });
  }

  return groups;
}

/**
 * Derives animated items from raw items.
 * In production, calls Date.now() for timestamps.
 * In tests, use `animatedItemsFrom(items, fixedNow)` directly.
 */
export function useAnimatedItems(items: Item[]): AnimatedItem[] {
  return useMemo(() => animatedItemsFrom(items, Date.now()), [items]);
}

/**
 * Groups animated items into turns.
 */
export function useTurnGrouping(animatedItems: AnimatedItem[]): TurnGroup[] {
  return useMemo(() => turnGroupsFrom(animatedItems), [animatedItems]);
}

export interface InspectorArtifact { key: string; title: string; display: Display; }

/** One Inspector artifact per tool Item that carries a display, in order. */
export function artifactsFrom(items: Item[]): InspectorArtifact[] {
  const out: InspectorArtifact[] = [];
  items.forEach((it, i) => {
    if (it.kind === "tool" && it.display) {
      const title = displayTitle(it.display) ?? it.name;
      out.push({ key: `art-${i}`, title, display: it.display });
    }
  });
  return out;
}

function displayTitle(d: Display): string | undefined {
  // every rich variant carries an optional title; older variants don't.
  const v = Object.values(d)[0] as { title?: string };
  return v && typeof v === "object" ? v.title : undefined;
}
