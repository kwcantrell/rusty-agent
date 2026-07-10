import { describe, it, expect, vi, beforeEach } from "vitest";

const { invoke, FakeChannel, channelInstances } = vi.hoisted(() => {
  const channelInstances: Array<{ onmessage?: (e: unknown) => void }> = [];
  class FakeChannel {
    onmessage?: (e: unknown) => void;
    constructor() { channelInstances.push(this); }
  }
  return { invoke: vi.fn(), FakeChannel, channelInstances };
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...a),
  Channel: FakeChannel,
}));

import { connect } from "../src/socket";
import type { Inbound } from "../src/wire";
import type { ConnectionStatus } from "../src/state";

beforeEach(() => {
  invoke.mockReset();
  channelInstances.length = 0;
  invoke.mockResolvedValue(undefined);
});

describe("ipc transport", () => {
  it("subscribes a channel and reports online once open", async () => {
    const frames: Inbound[] = [];
    const statuses: ConnectionStatus[] = [];
    connect({ onFrame: (f) => frames.push(f), onStatus: (s) => statuses.push(s) });
    await Promise.resolve();
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledWith("subscribe", expect.objectContaining({ channel: expect.any(FakeChannel) }));
    expect(statuses).toContain("open");
    expect(frames).toContainEqual({ v: 1, session_id: "", kind: "presence", online: true });
  });

  it("maps a token server event to an event frame", async () => {
    const frames: Inbound[] = [];
    connect({ onFrame: (f) => frames.push(f), onStatus: () => {} });
    await Promise.resolve();
    channelInstances[0].onmessage?.({ type: "token", text: "hi" });
    expect(frames).toContainEqual({ v: 1, session_id: "", kind: "event", payload: { type: "token", text: "hi" } });
  });

  it("maps an approval_request server event to an approval_request frame", async () => {
    const frames: Inbound[] = [];
    connect({ onFrame: (f) => frames.push(f), onStatus: () => {} });
    await Promise.resolve();
    channelInstances[0].onmessage?.({ type: "approval_request", id: "c0", summary: "run x" });
    expect(frames).toContainEqual(expect.objectContaining({ kind: "approval_request", id: "c0", summary: "run x" }));
  });

  it("routes user_input to the send_input command", () => {
    const sock = connect({ onFrame: () => {}, onStatus: () => {} });
    sock.send({ kind: "user_input", text: "hello" });
    expect(invoke).toHaveBeenCalledWith("send_input", { text: "hello" });
  });

  it("routes approval_response to the approve command", () => {
    const sock = connect({ onFrame: () => {}, onStatus: () => {} });
    sock.send({ kind: "approval_response", id: "c0", decision: "approve" });
    expect(invoke).toHaveBeenCalledWith("approve", { id: "c0", decision: "approve" });
  });

  it("routes a deny-with-feedback approval_response to the approve command", () => {
    const sock = connect({ onFrame: () => {}, onStatus: () => {} });
    sock.send({ kind: "approval_response", id: "c0", decision: { deny: { feedback: "why" } } });
    expect(invoke).toHaveBeenCalledWith("approve", { id: "c0", decision: { deny: { feedback: "why" } } });
  });

  it("dispatches settings_get result as a settings_state frame", async () => {
    invoke.mockReset();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "settings_get") {
        return Promise.resolve({ settings: { model: "m" }, workspace: "/w", api_key_set: false, hard_floor: [], discovered_skills: [] });
      }
      return Promise.resolve(undefined);
    });
    const frames: Inbound[] = [];
    const sock = connect({ onFrame: (f) => frames.push(f), onStatus: () => {} });
    sock.send({ kind: "settings_get" });
    await Promise.resolve();
    await Promise.resolve();
    expect(frames).toContainEqual(expect.objectContaining({ kind: "settings_state", workspace: "/w", api_key_set: false }));
  });
});
