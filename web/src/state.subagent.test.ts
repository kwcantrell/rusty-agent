import { describe, expect, it } from "vitest";
import { initialState, reduce, SUBAGENT_TRANSCRIPT_CAP } from "./state";
import type { Item } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

function tools(items: Item[]) {
  return items.filter((i): i is Extract<Item, { kind: "tool" }> => i.kind === "tool");
}

describe("subagent card assembly", () => {
  it("subagent_start attaches a running card with subagentType to the matching dispatch row", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore", role: "researcher" });
    const t = tools(s.items);
    expect(t).toHaveLength(1);
    expect(t[0].subagent).toMatchObject({ subagentType: "explore", role: "researcher", status: "running" });
  });
});

describe("subagent text/reasoning append routed by id", () => {
  it("keeps two interleaved delegations' deltas separate", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, { type: "tool_start", id: "c2", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c2", subagent_type: "plan" });
    s = red(s, { type: "subagent_text", id: "c1", text: "hello " });
    s = red(s, { type: "subagent_reasoning", id: "c2", text: "thinking " });
    s = red(s, { type: "subagent_text", id: "c2", text: "world" });
    s = red(s, { type: "subagent_text", id: "c1", text: "there" });
    const t = tools(s.items);
    const c1 = t.find((it) => it.id === "c1")!;
    const c2 = t.find((it) => it.id === "c2")!;
    expect(c1.subagent).toMatchObject({ text: "hello there", reasoning: "" });
    expect(c2.subagent).toMatchObject({ text: "world", reasoning: "thinking " });
  });
});

describe("duplicate call id across turns", () => {
  it("opens a NEW card when subagent_start reuses an id whose old card is done", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, { type: "subagent_text", id: "c1", text: "first run" });
    s = red(s, {
      type: "subagent_end", id: "c1", outcome: "completed",
      turns: 1, tool_calls: 0, duration_ms: 10,
    });
    // Second delegation reuses the same call id "c1".
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "plan" });
    const t = tools(s.items);
    expect(t).toHaveLength(2);
    expect(t[0].subagent).toMatchObject({ status: "done", text: "first run", subagentType: "explore" });
    expect(t[1].subagent).toMatchObject({ status: "running", text: "", subagentType: "plan" });
  });
});

describe("placeholder card", () => {
  it("subagent_text with no prior items creates a placeholder card holding the text", () => {
    let s = initialState([]);
    s = red(s, { type: "subagent_text", id: "orphan", text: "surviving text" });
    const t = tools(s.items);
    expect(t).toHaveLength(1);
    expect(t[0]).toMatchObject({ name: "dispatch_agent", id: "orphan", status: "running" });
    expect(t[0].subagent).toMatchObject({ text: "surviving text", status: "running" });
  });

  it("subagent_end landing on a placeholder (no tool_start ever arrived) finalizes the OUTER status too", () => {
    let s = initialState([]);
    s = red(s, { type: "subagent_text", id: "orphan", text: "surviving text" });
    s = red(s, {
      type: "subagent_end", id: "orphan", outcome: "completed",
      turns: 1, tool_calls: 0, duration_ms: 10,
    });
    const t = tools(s.items);
    expect(t).toHaveLength(1);
    // The OUTER status must flip to "done" — a placeholder never receives a
    // real tool_result, so leaving it "running" would pulse forever
    // (finding 2, 3B-2 review).
    expect(t[0].status).toBe("done");
    expect(t[0].subagent).toMatchObject({ status: "done", outcome: "completed" });
  });
});

describe("subagent transcript cap", () => {
  it("head-trims text to CAP code points and counts elided", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    const chunk1 = "a".repeat(SUBAGENT_TRANSCRIPT_CAP);
    const chunk2 = "b".repeat(100);
    s = red(s, { type: "subagent_text", id: "c1", text: chunk1 });
    s = red(s, { type: "subagent_text", id: "c1", text: chunk2 });
    const t = tools(s.items);
    const card = t[0].subagent!;
    expect(Array.from(card.text).length).toBe(SUBAGENT_TRANSCRIPT_CAP);
    expect(card.textElided).toBe(100);
    // Newest bytes are kept: the trailing "b"s survive, head "a"s were trimmed.
    expect(card.text.endsWith("b".repeat(100))).toBe(true);
  });
});

describe("subagent stream retry trim", () => {
  it("trims discarded_text_chars off the tail of the card transcript", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, { type: "subagent_text", id: "c1", text: "abcdef" });
    s = red(s, {
      type: "subagent_stream_retry", id: "c1",
      discarded_text_chars: 3, discarded_reasoning_chars: 0,
    });
    const t = tools(s.items);
    expect(t[0].subagent!.text).toBe("abc");
  });
});

describe("subagent cost accumulation", () => {
  it("accumulates child server_usage into the card and leaves the turn readout keyed by non-parented frames", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, {
      type: "server_usage", prompt_tokens: 10, completion_tokens: 5, turn: 1,
      cost_usd: 0.01, parent_id: "c1",
    });
    s = red(s, {
      type: "server_usage", prompt_tokens: 10, completion_tokens: 5, turn: 2,
      cost_usd: 0.01, parent_id: "c1",
    });
    const t = tools(s.items);
    expect(t[0].subagent).toMatchObject({ promptTokens: 20, completionTokens: 10, costUsd: 0.02 });
    expect(s.serverUsage).toBeNull();
    // Without parent_id, the readout updates as before.
    s = red(s, { type: "server_usage", prompt_tokens: 42, completion_tokens: 1, turn: 3 });
    expect(s.serverUsage).toMatchObject({ promptTokens: 42, turn: 3 });
  });
});

describe("finalize-on-done safety net", () => {
  it("finalizes a still-running card as status done / outcome unknown", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, { type: "done", reason: "stop" });
    const t = tools(s.items);
    expect(t[0].subagent).toMatchObject({ status: "done", outcome: "unknown" });
  });

  it("flips the OUTER status of a placeholder card but leaves a REAL running dispatch row untouched", () => {
    let s = initialState([]);
    // A placeholder card: subagent frames arrived with no matching tool_start.
    s = red(s, { type: "subagent_text", id: "orphan", text: "partial" });
    // A REAL dispatch row still awaiting its tool_result: tool_start fired,
    // subagent_start attached a card, but no tool_result and no subagent_end
    // has arrived yet.
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, { type: "done", reason: "stop" });
    const t = tools(s.items);
    const placeholder = t.find((it) => it.id === "orphan")!;
    const real = t.find((it) => it.id === "c1")!;
    expect(placeholder.status).toBe("done");
    expect(placeholder.subagent).toMatchObject({ status: "done", outcome: "unknown" });
    // The real row's card is also finalized as "unknown" (safety net), but its
    // OUTER status must stay "running" — tool_result correlation matches on
    // it, and a real tool_result may still be in flight.
    expect(real.status).toBe("running");
    expect(real.subagent).toMatchObject({ status: "done", outcome: "unknown" });
  });
});

describe("subagent_end fields", () => {
  it("sets outcome/stop/detail/stats fields", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = red(s, {
      type: "subagent_end", id: "c1", outcome: "timeout", stop: "max_turns",
      detail: "hit the ceiling", turns: 4, tool_calls: 7, duration_ms: 12345,
    });
    const t = tools(s.items);
    expect(t[0].subagent).toMatchObject({
      status: "done", outcome: "timeout", stop: "max_turns", detail: "hit the ceiling",
      turns: 4, toolCalls: 7, durationMs: 12345,
    });
  });
});

describe("no-typed-frames fallback", () => {
  it("a tool_start/tool_result pair with no subagent frames yields subagent === undefined", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "r1", name: "read_file", args: {} });
    s = red(s, {
      type: "tool_result", id: "r1", name: "read_file", status: "ok",
      duration_ms: 1, content: "data",
    });
    const t = tools(s.items);
    expect(t[0].subagent).toBeUndefined();
  });
});

describe("depth-2 delegation", () => {
  it("attaches subagent_start to a forwarded sub:dispatch_agent row", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "d1", name: "dispatch_agent", args: {} });
    s = red(s, {
      type: "tool_start", id: "sub3:c2", name: "sub:dispatch_agent", args: {}, parent_id: "c9",
    });
    s = red(s, { type: "subagent_start", id: "sub3:c2", subagent_type: "nested-explore" });
    const t = tools(s.items);
    const nested = t.find((it) => it.id === "sub3:c2")!;
    expect(nested).toMatchObject({ name: "sub:dispatch_agent", parentId: "c9" });
    expect(nested.subagent).toMatchObject({ subagentType: "nested-explore", status: "running" });
  });
});

describe("out-of-contract ordering (plan-review Finding 4)", () => {
  it("does not silently lose text accumulated before a late subagent_start", () => {
    let s = initialState([]);
    // subagent_text arrives with no live card: materializes a placeholder.
    s = red(s, { type: "subagent_text", id: "c1", text: "early text" });
    // A subagent_start for the same id now arrives — the placeholder card
    // is already running with a card, so this opens a second item rather
    // than overwriting; either way the earlier text must still be present.
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    const allText = tools(s.items)
      .map((it) => it.subagent?.text ?? "")
      .join("|");
    expect(allText).toContain("early text");
  });
});
