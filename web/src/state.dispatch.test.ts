import { describe, expect, it } from "vitest";
import { describeContext, initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

describe("session_stats after done", () => {
  it("leaves inTurn false, does not append a user item, and stores stats", () => {
    const stats = {
      turns: 1, prompt_tokens: 100, completion_tokens: 50, reasoning_tokens: 0,
      cached_tokens: 0, cost_usd: 0.01, tool_calls: 0, tools_ok: 0, tools_denied: 0,
      tools_error: 0, tools_timeout: 0, tools_panic: 0, tool_time_ms: 0,
      turn_time_ms: 500, context_events: 0, errors: 0,
    };
    // Start a turn, finish it with done, then receive session_stats
    let s = initialState(["hello"]);
    s = red(s, { type: "token", text: "hi" });
    s = red(s, { type: "done", reason: "stop" });
    expect(s.inTurn).toBe(false);
    const itemsBefore = s.items.length;
    s = red(s, { type: "session_stats", stats });
    expect(s.inTurn).toBe(false);
    expect(s.items.length).toBe(itemsBefore); // no phantom user item
    expect(s.stats).toEqual(stats);
  });
});

describe("error event closes the turn", () => {
  it("appends the error item and sets inTurn false", () => {
    let s = initialState([]);
    s = red(s, { type: "token", text: "partial" });
    expect(s.inTurn).toBe(true);
    s = red(s, { type: "error", message: "model overloaded" });
    expect(s.inTurn).toBe(false);
    const errors = s.items.filter((i) => i.kind === "error");
    expect(errors).toHaveLength(1);
    expect(errors[0]).toMatchObject({ kind: "error", message: "model overloaded" });
  });
});

describe("describeContext for offloaded events", () => {
  it("renders offloaded events by path, falling back to legacy id", () => {
    expect(describeContext("offloaded", { tool: "shell", path: "large_tool_results/1-c1" }))
      .toBe("offloaded shell result → large_tool_results/1-c1");
    // Pre-Phase-2 trace replay: old events carry a numeric id, no path.
    expect(describeContext("offloaded", { tool: "shell", id: 4 }))
      .toBe("offloaded shell result → #4");
  });
});

describe("sub-agent attribution", () => {
  it("correlates tool_result by id when two same-named tools run", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "a", name: "read_file", args: {} });
    s = red(s, { type: "tool_start", id: "b", name: "read_file", args: {} });
    s = red(s, { type: "tool_result", id: "a", name: "read_file", status: "ok", duration_ms: 1, content: "first" });
    const tools = s.items.filter((i) => i.kind === "tool");
    expect(tools[0]).toMatchObject({ id: "a", status: "done", content: "first" });
    expect(tools[1]).toMatchObject({ id: "b", status: "running" });
  });

  it("falls back to name-correlation for items without ids (old persisted state)", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", name: "legacy", args: {} }); // no id
    s = red(s, { type: "tool_result", id: "x", name: "legacy", status: "ok", duration_ms: 1, content: "c" });
    expect(s.items.find((i) => i.kind === "tool")).toMatchObject({ status: "done" });
  });

  it("stores parentId on attributed child rows", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "d1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "tool_start", id: "sub1:c1", name: "sub:read_file", args: {}, parent_id: "d1" });
    const child = s.items.filter((i) => i.kind === "tool")[1];
    expect(child).toMatchObject({ parentId: "d1", name: "sub:read_file" });
  });

  it("child server_usage does not touch the turn readout", () => {
    let s = initialState([]);
    s = red(s, { type: "server_usage", prompt_tokens: 10, completion_tokens: 1, turn: 2 });
    s = red(s, { type: "server_usage", prompt_tokens: 99, completion_tokens: 1, turn: 7, parent_id: "d1" });
    expect(s.serverUsage).toMatchObject({ promptTokens: 10, turn: 2 });
  });
});
