import { describe, it, expect } from "vitest";
import { reduce, initialState, animatedItemsFrom } from "./state";
import type { Inbound } from "./wire";

const ev = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

describe("reasoning events", () => {
  it("accumulates reasoning into a reasoning item, separate from the answer", () => {
    let s = initialState([]);
    s = reduce(s, { type: "user_send", text: "hi" });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "plan " }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "more" }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "answer" }) });
    const reasoning = s.items.find((i) => i.kind === "reasoning");
    const assistant = s.items.find((i) => i.kind === "assistant");
    expect(reasoning).toMatchObject({ kind: "reasoning", text: "plan more" });
    expect(assistant).toMatchObject({ kind: "assistant", text: "answer" });
  });
});

describe("streaming termination", () => {
  // Only the trailing block of a turn is still live. Deltas only ever extend the
  // last item, so any earlier reasoning/assistant block is provably finished.
  it("stops marking a reasoning block streaming once a later block exists", () => {
    let s = initialState([]);
    s = reduce(s, { type: "user_send", text: "hi" });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "thinking..." }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "done thinking" }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "done", reason: "stop" }) });
    const animated = animatedItemsFrom(s.items, 0);
    const reasoning = animated.find((i) => i.kind === "reasoning")!;
    expect(reasoning.streaming).toBe(false);
  });

  it("stops marking a preamble assistant message streaming after a tool call supersedes it", () => {
    let s = initialState([]);
    s = reduce(s, { type: "user_send", text: "write a script" });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "I'll create it." }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "tool_start", id: "c1", name: "write_file", args: {} }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "tool_result", id: "c1", name: "write_file", status: "ok", duration_ms: 3, content: "ok" }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "Created it." }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "done", reason: "stop" }) });
    const animated = animatedItemsFrom(s.items, 0);
    const assistants = animated.filter((i) => i.kind === "assistant");
    expect(assistants.map((a) => a.streaming)).toEqual([false, false]);
  });
});
