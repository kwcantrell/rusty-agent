import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";

// Force Tauri mode; the connected render is what we test.
vi.mock("../src/transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    sessionId: "11111111-1111-1111-1111-111111111111",
  }),
}));

// Mock the Tauri core IPC: invoke() resolves, and Channel captures instances so
// the test can push server events through `onmessage` (the IPC equivalent of the
// old window-injected WebSocket).
const { invoke, FakeChannel, channelInstances } = vi.hoisted(() => {
  const channelInstances: Array<{ onmessage?: (e: unknown) => void }> = [];
  class FakeChannel {
    onmessage?: (e: unknown) => void;
    constructor() { channelInstances.push(this); }
  }
  return { invoke: vi.fn(async (..._a: unknown[]) => null), FakeChannel, channelInstances };
});
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => invoke(...a),
  Channel: FakeChannel,
}));

beforeEach(() => {
  localStorage.clear();
  channelInstances.length = 0;
});

describe("App (Tauri mode)", () => {
  it("connects to the local bridge and renders streamed frames", async () => {
    const App = (await import("../src/App")).default;
    render(<App />);
    // resolveTransport() resolves a microtask later, then the connect effect
    // runs and subscribes a Channel.
    await waitFor(() => expect(channelInstances.length).toBeGreaterThan(0));
    const ch = channelInstances[0];
    act(() => {
      ch.onmessage?.({ type: "token", text: "hello world" });
      // Complete the stream so the assistant text is fully revealed.
      ch.onmessage?.({ type: "done", reason: "stop" });
    });
    expect(await screen.findByText("hello world")).toBeInTheDocument();
  });
});
