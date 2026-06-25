import { describe, it, expect, beforeEach } from "vitest";

describe("resolveTransport", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("returns a stable local session id (history key only)", async () => {
    const { resolveTransport } = await import("./transport");
    const a = await resolveTransport();
    expect(a.sessionId).toMatch(/[0-9a-f-]{36}/);
    const b = await resolveTransport();
    expect(b.sessionId).toBe(a.sessionId); // persisted in localStorage
  });

  it("does not expose a wsUrl", async () => {
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect((t as unknown as Record<string, unknown>).wsUrl).toBeUndefined();
  });
});
