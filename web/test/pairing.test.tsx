import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PairingScreen } from "../src/components/PairingScreen";

beforeEach(() => localStorage.clear());

describe("PairingScreen", () => {
  it("pairs with a code and reports the session", async () => {
    const onPaired = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => ({
      ok: true,
      json: async () => ({ session_id: "sess-1", session_token: "tok-1", agent_id: "a1" }),
    })) as unknown as typeof fetch);

    render(<PairingScreen onPaired={onPaired} />);
    await userEvent.type(screen.getByRole("textbox"), "123456");
    await userEvent.click(screen.getByRole("button", { name: /pair/i }));

    expect(onPaired).toHaveBeenCalledWith({ sessionId: "sess-1", token: "tok-1" });
    vi.unstubAllGlobals();
  });

  it("shows an error on a bad code", async () => {
    const onPaired = vi.fn();
    vi.stubGlobal("fetch", vi.fn(async () => ({ ok: false, status: 404, json: async () => ({ error: "invalid pairing code" }) })) as unknown as typeof fetch);
    render(<PairingScreen onPaired={onPaired} />);
    await userEvent.type(screen.getByRole("textbox"), "000000");
    await userEvent.click(screen.getByRole("button", { name: /pair/i }));
    expect(await screen.findByText(/invalid/i)).toBeInTheDocument();
    expect(onPaired).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });
});
