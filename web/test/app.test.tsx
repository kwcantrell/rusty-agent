import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, act } from "@testing-library/react";
import App from "../src/App";
import { saveSession } from "../src/storage";

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
  (window as unknown as { __WS__?: unknown }).__WS__ = TestWS;
});

describe("App", () => {
  it("with a stored token, connects and renders streamed frames", () => {
    saveSession("sess-1", "tok-1");
    render(<App />);
    act(() => { TestWS.last!.onopen?.(); });
    act(() => {
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: "sess-1", kind: "presence", online: true }) });
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: "sess-1", kind: "event", payload: { type: "token", text: "hello world" } }) });
      // Complete the stream so the assistant text is fully revealed (no longer mid-stream
      // animating char-by-char via useStreamingText).
      TestWS.last!.onmessage?.({ data: JSON.stringify({ v: 1, session_id: "sess-1", kind: "event", payload: { type: "done", reason: "stop" } }) });
    });
    expect(screen.getByText(/agent online/i)).toBeInTheDocument();
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });

  it("without a token, shows the pairing screen", () => {
    render(<App />);
    expect(screen.getByRole("button", { name: /pair/i })).toBeInTheDocument();
  });
});
