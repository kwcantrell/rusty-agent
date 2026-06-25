import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, act, waitFor } from "@testing-library/react";

// Force Tauri mode and a fixed bridge URL; the connected render is what we test.
vi.mock("../src/transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
  }),
}));
// App dynamically imports invoke() for llama_health/get_workspace; stub it.
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

// A controllable WebSocket the App will use (App reads a window-injected impl in tests).
class TestWS {
  static last: TestWS | null = null;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  readyState = 1;
  sent: string[] = [];
  constructor(public url: string) { TestWS.last = this; }
  send(d: string) { this.sent.push(d); }
  close() { this.readyState = 3; this.onclose?.(); }
}

beforeEach(() => {
  localStorage.clear();
  TestWS.last = null;
  (window as unknown as { __WS__?: unknown }).__WS__ = TestWS;
});

describe("App (Tauri mode)", () => {
  it("connects to the local bridge and renders streamed frames", async () => {
    const App = (await import("../src/App")).default;
    render(<App />);
    // resolveTransport() resolves a microtask later, then the connect effect runs.
    await waitFor(() => expect(TestWS.last).not.toBeNull());
    const SID = "11111111-1111-1111-1111-111111111111";
    act(() => { TestWS.last!.onopen?.(); });
    act(() => {
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: SID, kind: "event", payload: { type: "token", text: "hello world" } }) });
      // Complete the stream so the assistant text is fully revealed.
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: SID, kind: "event", payload: { type: "done", reason: "stop" } }) });
    });
    expect(await screen.findByText("hello world")).toBeInTheDocument();
  });
});
