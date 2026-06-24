import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

describe("resolveTransport", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    localStorage.clear();
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  it("uses the local bridge URL and skips pairing in Tauri mode", async () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    invokeMock.mockResolvedValue("ws://127.0.0.1:54321/agent");
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(invokeMock).toHaveBeenCalledWith("get_local_ws_url");
    expect(t.wsUrl).toBe("ws://127.0.0.1:54321/agent");
    expect(t.needsPairing).toBe(false);
    expect(t.sessionId).toMatch(/[0-9a-f-]{36}/);
  });

  it("requires pairing in browser mode", async () => {
    const { resolveTransport } = await import("./transport");
    const t = await resolveTransport();
    expect(t.needsPairing).toBe(true);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
