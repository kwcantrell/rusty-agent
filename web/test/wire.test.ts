import { describe, it, expect } from "vitest";
import { parseInbound } from "../src/wire";

describe("parseInbound", () => {
  it("parses a token event", () => {
    const f = parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi" } }));
    expect(f).toEqual({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi" } });
  });
  it("parses a tool_result with a Terminal display", () => {
    const f = parseInbound(JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "execute_command", content: "exit=0",
        display: { Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } } } }));
    expect(f?.kind).toBe("event");
    if (f?.kind === "event" && f.payload.type === "tool_result") {
      expect(f.payload.display).toEqual({ Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } });
    } else throw new Error("wrong shape");
  });
  it("parses a Diff display", () => {
    const f = parseInbound(JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "edit_file", content: "ok",
        display: { Diff: { path: "a.txt", before: "x\n", after: "y\n" } } } }));
    if (f?.kind === "event" && f.payload.type === "tool_result") {
      expect(f.payload.display).toEqual({ Diff: { path: "a.txt", before: "x\n", after: "y\n" } });
    } else throw new Error("wrong shape");
  });
  it("parses approval_request and presence", () => {
    const a = parseInbound(JSON.stringify({ v: 1, session_id: "s", id: "c0", kind: "approval_request", summary: "run x", command: "x" }));
    expect(a).toMatchObject({ kind: "approval_request", id: "c0", summary: "run x", command: "x" });
    const p = parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "presence", online: true }));
    expect(p).toMatchObject({ kind: "presence", online: true });
  });
  it("returns null on malformed json or unknown kind", () => {
    expect(parseInbound("{not json")).toBeNull();
    expect(parseInbound(JSON.stringify({ v: 1, session_id: "s", kind: "mystery" }))).toBeNull();
  });
});
