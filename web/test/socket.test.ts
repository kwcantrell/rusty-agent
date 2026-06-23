import { describe, it, expect, vi } from "vitest";
import { connect } from "../src/socket";
import type { Inbound } from "../src/wire";
import type { ConnectionStatus } from "../src/state";

class FakeWS {
  static instances: FakeWS[] = [];
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((e: { data: string }) => void) | null = null;
  readyState = 0;
  sent: string[] = [];
  url: string;
  constructor(url: string) { this.url = url; FakeWS.instances.push(this); }
  send(d: string) { this.sent.push(d); }
  close() { this.readyState = 3; this.onclose?.(); }
  open() { this.readyState = 1; this.onopen?.(); }
  message(o: unknown) { this.onmessage?.({ data: JSON.stringify(o) }); }
}

describe("socket", () => {
  it("reports status open and delivers parsed frames", () => {
    FakeWS.instances = [];
    const frames: Inbound[] = []; const statuses: ConnectionStatus[] = [];
    connect("ws://x/browser?token=t", { onFrame: (f) => frames.push(f), onStatus: (s) => statuses.push(s) },
      { WebSocketImpl: FakeWS as unknown as typeof WebSocket });
    const ws = FakeWS.instances[0];
    ws.open();
    ws.message({ v: 1, session_id: "s", kind: "presence", online: true });
    expect(statuses).toContain("open");
    expect(frames).toEqual([{ v: 1, session_id: "s", kind: "presence", online: true }]);
  });

  it("doubles backoff on repeated closes and resets on open", () => {
    vi.useFakeTimers();
    FakeWS.instances = [];
    connect("ws://x/browser?token=t", { onFrame: () => {}, onStatus: () => {} },
      { WebSocketImpl: FakeWS as unknown as typeof WebSocket, backoffMs: 10 });
    FakeWS.instances[0].open();        // backoff reset to 10
    FakeWS.instances[0].close();       // unexpected -> reconnect in 10, next backoff 20
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(2);
    FakeWS.instances[1].close();       // unexpected, no open -> reconnect in 20
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(2); // NOT yet: proves delay doubled to 20
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(3); // fired at 20
    FakeWS.instances[2].open();        // resets backoff to 10
    FakeWS.instances[2].close();       // reconnect in 10 again
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(4); // proves reset to base
    vi.useRealTimers();
  });

  it("reconnects on unexpected close but not after a deliberate close()", () => {
    vi.useFakeTimers();
    FakeWS.instances = [];
    const handle = connect("ws://x/browser?token=t", { onFrame: () => {}, onStatus: () => {} },
      { WebSocketImpl: FakeWS as unknown as typeof WebSocket, backoffMs: 10 });
    FakeWS.instances[0].open();
    FakeWS.instances[0].close(); // unexpected
    vi.advanceTimersByTime(10);
    expect(FakeWS.instances.length).toBe(2); // reconnected
    handle.close(); // deliberate
    vi.advanceTimersByTime(1000);
    expect(FakeWS.instances.length).toBe(2); // no further reconnect
    vi.useRealTimers();
  });
});
