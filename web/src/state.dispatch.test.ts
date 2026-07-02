import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

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
