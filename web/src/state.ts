import { useMemo } from "react";
import type { ApprovalOrigin, Display, Inbound, ParkedRun, RuntimeSettings, SessionStats } from "./wire";
import { displayDesignId } from "./designStore";

export type ConnectionStatus = "connecting" | "open" | "closed" | "error";

/** Live per-delegation card state (spec 3B-2 §2.4). Attached to the
 *  dispatch_agent tool item whose id equals the delegation id. */
export interface SubagentCard {
  subagentType: string;
  role?: string;
  status: "running" | "done";
  text: string;
  reasoning: string;
  /** Code points head-trimmed off text/reasoning by the transcript cap. */
  textElided: number;
  reasoningElided: number;
  outcome?: string;
  stop?: string;
  detail?: string;
  /** Accumulated from child server_usage frames (parent_id === delegation id). */
  promptTokens: number;
  completionTokens: number;
  costUsd: number;
  turns?: number;
  toolCalls?: number;
  durationMs?: number;
  /** True while a nested approval_request attributed to this delegation is
   *  pending an answer (spec 4B-1). */
  waitingApproval?: boolean;
}

export type Item =
  | { kind: "user"; text: string }
  | { kind: "assistant"; text: string; done?: string }
  | { kind: "reasoning"; text: string }
  | { kind: "tool"; name: string; args: unknown; status: "running" | "done";
      id?: string; parentId?: string; subagent?: SubagentCard;
      content?: string; display?: Display; resultStatus?: string; durationMs?: number;
      /** Set only by `placeholderCardItem`: this row was materialized from a
       *  subagent frame, not a real `tool_start` — no `tool_result` will ever
       *  arrive for it, so its outer `status` must be flipped explicitly
       *  wherever its card finalizes (finding 2, 3B-2 review). */
      placeholder?: true }
  | { kind: "context"; text: string }
  | { kind: "error"; message: string };

/** The tool-item shape of `Item`, narrowed for card helpers. */
type ToolItem = Extract<Item, { kind: "tool" }>;

function isToolItem(it: Item): it is ToolItem {
  return it.kind === "tool";
}

export interface PendingApproval {
  id: string;
  summary: string;
  command?: string;
  display?: Display;
  origin?: ApprovalOrigin;
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
  parkedRuns: ParkedRun[];
}

export type Action =
  | { type: "reset"; userMsgs: string[] }
  | { type: "frame"; frame: Inbound }
  | { type: "user_send"; text: string }
  | { type: "approval_sent" }
  | { type: "status"; status: ConnectionStatus }
  | { type: "dismiss_sandbox_banner" }
  | { type: "dismiss_parked_banner" };

export function initialState(userMsgs: string[]): ConversationState {
  return { items: [], pendingApproval: null, usage: null, serverUsage: null, online: false, status: "connecting", userMsgs, turnIndex: 0, inTurn: false,
    settings: null, settingsMeta: null, settingsError: null, sandboxDegraded: null, stats: null, parkedRuns: [] };
}

/** One-line human summary of a context-management event, keyed by kind. */
export function describeContext(kind: string, detail: Record<string, unknown>): string {
  switch (kind) {
    case "offloaded": return `offloaded ${detail.tool} result → ${detail.path ?? `#${detail.id}`}`;
    case "compacted":
      return `compacted ${detail.turns_replaced} turns: ${detail.tokens_before} → ${detail.tokens_after} tokens`;
    case "compaction_failed": return `compaction failed: ${detail.reason}`;
    case "evicted": return `evicted ${detail.messages} messages (~${detail.est_tokens} tokens)`;
    case "overflow_recovery":
      return "context overflow: compacted and retried";
    default: return kind;
  }
}

/**
 * Trim `chars` code points off the tail of the last item of `kind`, in place.
 * Removes the item entirely if it empties. Counts by Unicode code point to
 * match the core's `chars().count()` (the source of `discarded_*_chars`).
 */
function trimTrailing(items: Item[], kind: "assistant" | "reasoning", chars: number): void {
  if (chars <= 0) return;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === kind) {
      const cps = Array.from(it.text);
      const kept = cps.slice(0, Math.max(0, cps.length - chars)).join("");
      if (kept.length === 0) {
        items.splice(i, 1);
      } else {
        items[i] = { ...it, text: kept };
      }
      return;
    }
  }
}

/** Per-card transcript budget, in code points (spec §2.4: a runaway child
 *  must not grow a single React string unboundedly; append is a full copy). */
export const SUBAGENT_TRANSCRIPT_CAP = 30000;

function freshCard(subagentType: string, role?: string): SubagentCard {
  return { subagentType, role, status: "running", text: "", reasoning: "",
    textElided: 0, reasoningElided: 0, promptTokens: 0, completionTokens: 0, costUsd: 0 };
}

/** Append with head-trim: keep the newest CAP code points, count what fell off. */
function appendCapped(cur: string, elided: number, delta: string): { s: string; elided: number } {
  const cps = Array.from(cur + delta);
  if (cps.length <= SUBAGENT_TRANSCRIPT_CAP) return { s: cur + delta, elided };
  const overflow = cps.length - SUBAGENT_TRANSCRIPT_CAP;
  return { s: cps.slice(overflow).join(""), elided: elided + overflow };
}

/** Trim `chars` code points off a card transcript tail (child stream retry). */
function trimTail(s: string, chars: number): string {
  if (chars <= 0) return s;
  const cps = Array.from(s);
  return cps.slice(0, Math.max(0, cps.length - chars)).join("");
}

/** Find the newest tool item with this delegation id whose card can still
 *  receive frames (running card, or bare running dispatch row). Returns -1
 *  when only done cards (or nothing) match — caller creates a new item
 *  (placeholder rule, spec §2.4 / gate G3). */
function findLiveCardIndex(items: Item[], id: string): number {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (isToolItem(it) && it.id === id &&
        (it.subagent ? it.subagent.status === "running" : it.status === "running")) {
      return i;
    }
  }
  return -1;
}

/** Placeholder item for frames that matched nothing (mid-run reload, or a
 *  reused call id whose old card is done). */
function placeholderCardItem(id: string, card: SubagentCard): Item {
  return { kind: "tool", name: "dispatch_agent", args: {}, status: "running", id,
    subagent: card, placeholder: true };
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
      return {
        ...state,
        pendingApproval: null,
        items: state.items.map((it) =>
          isToolItem(it) && it.subagent?.waitingApproval
            ? { ...it, subagent: { ...it.subagent, waitingApproval: false } }
            : it,
        ),
      };
    case "dismiss_sandbox_banner":
      return { ...state, sandboxDegraded: null };
    case "dismiss_parked_banner":
      return { ...state, parkedRuns: [] };
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
    let items = state.items;
    if (frame.origin) {
      items = [...items];
      const i = findLiveCardIndex(items, frame.origin.delegation_id);
      const it = i >= 0 ? items[i] : undefined;
      if (it && isToolItem(it) && it.subagent) {
        items[i] = { ...it, subagent: { ...it.subagent, waitingApproval: true } };
      }
    }
    return { ...state, items, pendingApproval: {
      id: frame.id, summary: frame.summary, command: frame.command,
      display: frame.display, origin: frame.origin } };
  }
  if (frame.kind === "parked_runs") {
    return { ...state, parkedRuns: frame.runs };
  }
  if (frame.kind === "approval_resolved") {
    // id-guarded: a retraction for an overwritten OLD id must not clear a
    // newer prompt (arch review finding 8 verified this guard sufficient).
    if (state.pendingApproval?.id !== frame.id) return state;
    return {
      ...state,
      pendingApproval: null,
      items: state.items.map((it) =>
        isToolItem(it) && it.subagent?.waitingApproval
          ? { ...it, subagent: { ...it.subagent, waitingApproval: false } }
          : it,
      ),
    };
  }
  if (frame.kind === "resumed") {
    return {
      ...state,
      parkedRuns: state.parkedRuns.filter((r) => r.session_id !== frame.resumed_session_id),
    };
  }
  // frame.kind === "event"
  const p = frame.payload;
  // Session stats arrive after `done`; they must not open a new turn.
  if (p.type === "session_stats") {
    return { ...state, stats: p.stats };
  }
  const s = startTurn(state);
  switch (p.type) {
    case "usage":
      return { ...s, usage: { promptTokens: p.prompt_tokens, contextLimit: p.context_limit, turn: p.turn, maxTurns: p.max_turns } };
    // The breakdown only needs the prompt total, so we intentionally keep only
    // promptTokens here and drop completion_tokens. Revisit if a chart needs it.
    case "server_usage": {
      // A sub-agent's usage frame must not flicker the parent turn readout;
      // it instead accumulates into its delegation card (spec 3B-2 §2.4) —
      // its tokens still land in session_stats (spec E5/E6c).
      if (p.parent_id) {
        const items = [...s.items];
        const i = findLiveCardIndex(items, p.parent_id);
        if (i < 0) return s;
        const it = items[i] as ToolItem;
        if (!it.subagent) return s;
        items[i] = { ...it, subagent: { ...it.subagent,
          promptTokens: it.subagent.promptTokens + p.prompt_tokens,
          completionTokens: it.subagent.completionTokens + p.completion_tokens,
          costUsd: it.subagent.costUsd + (p.cost_usd ?? 0) } };
        return { ...s, items };
      }
      return { ...s, serverUsage: { promptTokens: p.prompt_tokens, turn: p.turn } };
    }
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
      return { ...s, items: [...s.items, { kind: "tool", id: p.id, parentId: p.parent_id,
        name: p.name, args: p.args, status: "running" }] };
    case "tool_result": {
      const items = [...s.items];
      for (let i = items.length - 1; i >= 0; i--) {
        const it = items[i];
        // id-first correlation (parallel same-named child tools); name-fallback
        // only for tool items from an old SERVER that omitted `id` on tool_start
        // frames (restored history never contains tool items, so it's not that).
        if (it.kind === "tool" && it.status === "running" &&
            (it.id !== undefined ? it.id === p.id : it.name === p.name)) {
          items[i] = { ...it, status: "done", content: p.content, display: p.display,
            resultStatus: p.status, durationMs: p.duration_ms };
          break;
        }
      }
      return { ...s, items };
    }
    case "context":
      return { ...s, items: [...s.items, { kind: "context", text: describeContext(p.kind, p.detail) }] };
    case "sandbox_degraded":
      return { ...s, sandboxDegraded: { mechanism: p.mechanism, reason: p.reason } };
    case "stream_retry": {
      // A mid-stream failure abandoned the in-flight partial answer before a
      // retry re-streams. Tokens only ever extend the LAST item of a kind (see
      // the comment near isStreamingItem), so the discarded chars are exactly
      // the tail of the last assistant/reasoning item — trim them off, dropping
      // an item that empties completely.
      const items = [...s.items];
      trimTrailing(items, "assistant", p.discarded_text_chars);
      trimTrailing(items, "reasoning", p.discarded_reasoning_chars);
      return { ...s, items };
    }
    case "error":
      return { ...s, inTurn: false, items: [...s.items, { kind: "error", message: p.message }] };
    case "subagent_start": {
      const items = [...s.items];
      const i = findLiveCardIndex(items, p.id);
      const it = i >= 0 ? items[i] : undefined;
      if (it && isToolItem(it) && !it.subagent) {
        items[i] = { ...it, subagent: freshCard(p.subagent_type, p.role) };
      } else {
        // Reused call id landing on a live card, or no match at all → new card.
        items.push(placeholderCardItem(p.id, freshCard(p.subagent_type, p.role)));
      }
      return { ...s, items };
    }
    case "subagent_text":
    case "subagent_reasoning": {
      const items = [...s.items];
      let i = findLiveCardIndex(items, p.id);
      let it = i >= 0 ? items[i] : undefined;
      if (!it || !isToolItem(it) || !it.subagent) {
        // Placeholder rule: a frame with no live card materializes one so a
        // mid-run reload doesn't silently drop the delegation (gate G3).
        items.push(placeholderCardItem(p.id, freshCard("sub-agent")));
        i = items.length - 1;
        it = items[i] as ToolItem;
      }
      const card = { ...(it as ToolItem).subagent! };
      if (p.type === "subagent_text") {
        const r = appendCapped(card.text, card.textElided, p.text);
        card.text = r.s; card.textElided = r.elided;
      } else {
        const r = appendCapped(card.reasoning, card.reasoningElided, p.text);
        card.reasoning = r.s; card.reasoningElided = r.elided;
      }
      items[i] = { ...(it as ToolItem), subagent: card };
      return { ...s, items };
    }
    case "subagent_stream_retry": {
      const items = [...s.items];
      const i = findLiveCardIndex(items, p.id);
      if (i < 0) return s;
      const it = items[i] as ToolItem;
      if (!it.subagent) return s;
      items[i] = { ...it, subagent: { ...it.subagent,
        text: trimTail(it.subagent.text, p.discarded_text_chars),
        reasoning: trimTail(it.subagent.reasoning, p.discarded_reasoning_chars) } };
      return { ...s, items };
    }
    case "subagent_end": {
      const items = [...s.items];
      let i = findLiveCardIndex(items, p.id);
      let it = i >= 0 ? items[i] : undefined;
      if (!it || !isToolItem(it) || !it.subagent) {
        items.push(placeholderCardItem(p.id, freshCard("sub-agent")));
        i = items.length - 1;
        it = items[i];
      }
      const ti = it as ToolItem;
      items[i] = { ...ti, subagent: { ...ti.subagent!, status: "done",
        outcome: p.outcome, stop: p.stop, detail: p.detail,
        turns: p.turns, toolCalls: p.tool_calls, durationMs: p.duration_ms,
        waitingApproval: false },
        // Placeholder rows never get a real tool_result (spec §2.4 / gate G3),
        // so the outer status would otherwise pulse "running" forever
        // (finding 2, 3B-2 review). A REAL dispatch row awaiting its
        // tool_result must keep outer status "running" — that correlation
        // matches on it — so only flip placeholders.
        ...(ti.placeholder ? { status: "done" as const } : {}) };
      return { ...s, items };
    }
    case "done": {
      const items = [...s.items];
      const last = items[items.length - 1];
      if (last && last.kind === "assistant" && last.done === undefined) {
        items[items.length - 1] = { ...last, done: p.reason };
      }
      // Safety net: a card still running at run end lost its End somewhere —
      // finalize as "unknown" so nothing spins forever (spec §2.4). Only
      // placeholder rows get their OUTER status flipped here too — a real
      // dispatch row awaiting its tool_result must keep outer status
      // "running" since tool_result correlation matches on it (finding 2,
      // 3B-2 review); a placeholder never gets a real tool_result, so its
      // outer status would otherwise pulse "running" forever.
      for (let i = 0; i < items.length; i++) {
        const it = items[i];
        if (it.kind === "tool" && it.subagent?.status === "running") {
          items[i] = { ...it, subagent: { ...it.subagent, status: "done", outcome: "unknown" },
            ...(it.placeholder ? { status: "done" as const } : {}) };
        }
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
    if (it.kind === "tool" && it.display && displayDesignId(it.display) === null) {
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
