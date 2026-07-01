import { describe, it, expect } from "vitest";
import { reduce, initialState } from "./state";
import type { Inbound } from "./wire";

const ev = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as unknown as Inbound);

const settings = (sandbox_degraded: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "settings_state", settings: {}, workspace: "/w",
     api_key_set: true, hard_floor: [], discovered_skills: [], sandbox_degraded } as unknown as Inbound);

describe("sandbox degraded banner state", () => {
  it("sets posture from a settings_state frame (connect time)", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: settings({ mechanism: "docker", reason: "no daemon" }) });
    expect(s.sandboxDegraded).toEqual({ mechanism: "docker", reason: "no daemon" });
  });

  it("clears posture when settings_state reports healthy", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: settings({ mechanism: "docker", reason: "x" }) });
    s = reduce(s, { type: "frame", frame: settings(undefined) });
    expect(s.sandboxDegraded).toBeNull();
  });

  it("sets posture from a run-start sandbox_degraded event", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    expect(s.sandboxDegraded).toEqual({ mechanism: "docker", reason: "no daemon" });
  });

  it("dismiss clears the banner", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    s = reduce(s, { type: "dismiss_sandbox_banner" });
    expect(s.sandboxDegraded).toBeNull();
  });

  it("reset clears the banner", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: ev({ type: "sandbox_degraded", mechanism: "docker", reason: "no daemon" }) });
    s = reduce(s, { type: "reset", userMsgs: [] });
    expect(s.sandboxDegraded).toBeNull();
  });
});
