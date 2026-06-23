import { describe, it, expect } from "vitest";
import { initialState, reduce, type ConversationState } from "../src/state";
import type { Inbound } from "../src/wire";

function frame(f: Inbound) { return { type: "frame", frame: f } as const; }
function run(actions: Parameters<typeof reduce>[1][], userMsgs: string[] = []): ConversationState {
  return actions.reduce(reduce, initialState(userMsgs));
}

describe("reducer", () => {
  it("accumulates streamed tokens into one assistant item", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "Hel" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "lo" } }),
    ]);
    expect(s.items).toEqual([{ kind: "assistant", text: "Hello" }]);
  });

  it("correlates tool_result to the running tool of the same name", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_start", name: "execute_command", args: {} } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_result", name: "execute_command", content: "exit=0" } }),
    ]);
    expect(s.items).toEqual([{ kind: "tool", name: "execute_command", args: {}, status: "done", content: "exit=0", display: undefined }]);
  });

  it("sets and clears the pending approval", () => {
    let s = run([frame({ v: 1, session_id: "s", id: "c0", kind: "approval_request", summary: "run x", command: "x" })]);
    expect(s.pendingApproval).toMatchObject({ id: "c0", summary: "run x" });
    s = reduce(s, { type: "approval_sent" });
    expect(s.pendingApproval).toBeNull();
  });

  it("tracks presence and closes a turn on done", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "presence", online: true }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "ok" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ]);
    expect(s.online).toBe(true);
    expect(s.items).toEqual([{ kind: "assistant", text: "ok", done: "stop" }]);
  });

  it("reset-and-replay reconstructs history with user messages interleaved by turn", () => {
    // Two stored user messages -> they head turn 0 and turn 1.
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "A" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "B" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ], ["q1", "q2"]);
    expect(s.items).toEqual([
      { kind: "user", text: "q1" },
      { kind: "assistant", text: "A", done: "stop" },
      { kind: "user", text: "q2" },
      { kind: "assistant", text: "B", done: "stop" },
    ]);
  });

  it("user_send pushes the user item and is not double-emitted by the following turn", () => {
    const s = run([
      { type: "user_send", text: "hello" },
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "token", text: "hi back" } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "done", reason: "stop" } }),
    ]);
    expect(s.items).toEqual([
      { kind: "user", text: "hello" },
      { kind: "assistant", text: "hi back", done: "stop" },
    ]);
  });
});
