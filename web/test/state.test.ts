import { describe, it, expect } from "vitest";
import { initialState, reduce, type ConversationState } from "../src/state";
import type { Inbound, RuntimeSettings } from "../src/wire";

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
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_start", id: "c1", name: "execute_command", args: {} } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "tool_result", id: "c1", name: "execute_command", status: "ok", duration_ms: 12, content: "exit=0" } }),
    ]);
    expect(s.items).toEqual([{ kind: "tool", id: "c1", parentId: undefined, name: "execute_command", args: {}, status: "done", content: "exit=0", display: undefined, resultStatus: "ok", durationMs: 12 }]);
  });

  it("stores the latest usage event and clears it on reset", () => {
    let s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1200, context_limit: 8000, turn: 1, max_turns: 20 } }),
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 1500, context_limit: 8000, turn: 2, max_turns: 20 } }),
    ]);
    expect(s.usage).toEqual({ promptTokens: 1500, contextLimit: 8000, turn: 2, maxTurns: 20 });
    s = reduce(s, { type: "reset", userMsgs: [] });
    expect(s.usage).toBeNull();
  });

  it("does not add usage events to the transcript", () => {
    const s = run([
      frame({ v: 1, session_id: "s", kind: "event", payload: { type: "usage", prompt_tokens: 100, context_limit: 8000, turn: 1, max_turns: 20 } }),
    ]);
    expect(s.items).toEqual([]);
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

  it("settings_state stores settings + meta and clears error", () => {
    const s: RuntimeSettings = {
      backend: "openai", base_url: "u", model: "m", protocol: "native",
      command_allowlist: [], command_denylist: [], temperature: 0.2,
      max_tokens: 2048, max_turns: 25, context_limit: 8192,
      top_p: null, top_k: null, min_p: null, presence_penalty: null, repeat_penalty: null,
      enable_thinking: false, preserve_thinking: false, memory: true,
      skills_dirs: [], active_skills: [],
      trace: false, trace_dir: null, trace_max_mb: 64,
      system_prompt_override: null,
    };
    let st = initialState([]);
    st = reduce(st, { type: "frame", frame: { v: 1, session_id: "x", kind: "settings_error", message: "old" } });
    expect(st.settingsError).toBe("old");
    st = reduce(st, { type: "frame", frame: {
      v: 1, session_id: "x", kind: "settings_state", settings: s,
      workspace: "/w", api_key_set: true, hard_floor: ["sudo"],
      discovered_skills: [] } });
    expect(st.settings?.model).toBe("m");
    expect(st.settingsMeta?.workspace).toBe("/w");
    expect(st.settingsMeta?.apiKeySet).toBe(true);
    expect(st.settingsMeta?.hardFloor).toEqual(["sudo"]);
    expect(st.settingsError).toBeNull();
  });

  it("settings_error sets the error message", () => {
    let st = initialState([]);
    st = reduce(st, { type: "frame", frame: { v: 1, session_id: "x", kind: "settings_error", message: "nope" } });
    expect(st.settingsError).toBe("nope");
  });
});
