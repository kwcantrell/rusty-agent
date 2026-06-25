import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

vi.mock("./transport", () => ({
  isTauri: () => true,
  resolveTransport: async () => ({
    wsUrl: "ws://127.0.0.1:5/agent",
    sessionId: "11111111-1111-1111-1111-111111111111",
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

  it("skips the non-Tauri notice and connects to the local bridge", async () => {
    const App = (await import("./App")).default;
    render(<App />);
    // In Tauri mode the app connects to the local bridge; the "desktop app"
    // notice (shown only outside Tauri) must never appear.
    await waitFor(() => {
      expect(screen.queryByText(/desktop app/i)).toBeNull();
    });
    // Give the resolveTransport() microtasks a tick; the notice stays absent.
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText(/desktop app/i)).toBeNull();
  });
});
