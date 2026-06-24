import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("./transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
    needsPairing: false,
  }),
}));

// A no-op socket so connect() doesn't open a real WebSocket in jsdom.
vi.mock("./socket", () => ({
  connect: () => ({ send: vi.fn(), close: vi.fn() }),
}));

// App dynamically imports invoke() for get_workspace; stub it so no real
// Tauri internals are touched.
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => null }));

describe("App in Tauri mode", () => {
  beforeEach(() => localStorage.clear());

  it("skips the pairing screen and shows the main UI", async () => {
    const App = (await import("./App")).default;
    render(<App />);
    // PairingScreen renders <h1>Pair with your agent</h1>; in Tauri mode it must
    // never appear (we connect to the local bridge without pairing).
    await waitFor(() => {
      expect(screen.queryByText(/pair with your agent/i)).toBeNull();
    });
    // Give the resolveTransport() microtasks a tick; the heading stays absent.
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText(/pair with your agent/i)).toBeNull();
  });
});
