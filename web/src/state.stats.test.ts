import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

describe("cluster-2 observability frames", () => {
  it("stores session_stats", () => {
    const stats = { turns: 2, prompt_tokens: 300, completion_tokens: 90,
      reasoning_tokens: 10, cached_tokens: 60, cost_usd: 0.05, tool_calls: 3,
      tools_ok: 2, tools_denied: 0, tools_error: 1, tools_timeout: 0, tools_panic: 0,
      tool_time_ms: 900, turn_time_ms: 1200, context_events: 1, errors: 1 };
    const s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "session_stats", stats }) });
    expect(s.stats).toEqual(stats);
  });

  it("appends a context marker item", () => {
    const s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "context", kind: "compacted",
        detail: { turns_replaced: 3, tokens_before: 900, tokens_after: 200 } }) });
    const last = s.items[s.items.length - 1];
    expect(last.kind).toBe("context");
    expect((last as { text: string }).text).toContain("compacted 3 turns");
  });

  it("marks a failed tool result with status and duration", () => {
    let s = reduce(initialState([]), { type: "frame",
      frame: frame({ type: "tool_start", id: "c1", name: "read_file", args: {} }) });
    s = reduce(s, { type: "frame",
      frame: frame({ type: "tool_result", id: "c1", name: "read_file",
        status: "timeout", duration_ms: 60000, content: "ERROR: …" }) });
    const tool = s.items.find((i) => i.kind === "tool") as
      { resultStatus?: string; durationMs?: number };
    expect(tool.resultStatus).toBe("timeout");
    expect(tool.durationMs).toBe(60000);
  });

  it("leaves state unchanged on an unknown event type (forward compat)", () => {
    const before = initialState([]);
    const after = reduce(before, { type: "frame",
      frame: frame({ type: "some_future_event", anything: true }) });
    expect(after).toEqual(before);
  });
});
