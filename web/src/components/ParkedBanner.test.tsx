import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ParkedBanner } from "./ParkedBanner";

describe("ParkedBanner", () => {
  it("lists parked runs with ask counts and marks unresumable ones", () => {
    render(<ParkedBanner
      runs={[
        { session_id: "100-aaaaaaaa", workspace: "/w", created_ms: 5, asks: 2 },
        { session_id: "200-bbbbbbbb", workspace: "/x", created_ms: 6, asks: 0, error: "checkpoint unreadable" },
      ]}
      onDismiss={() => {}}
    />);
    expect(screen.getByText(/100-aaaaaaaa/)).toBeTruthy();
    expect(screen.getByText(/2 approvals? waiting/i)).toBeTruthy();
    expect(screen.getByText(/checkpoint unreadable/)).toBeTruthy();
  });

  it("dismisses", () => {
    const onDismiss = vi.fn();
    render(<ParkedBanner runs={[{ session_id: "1", workspace: "/w", created_ms: 5, asks: 1 }]} onDismiss={onDismiss} />);
    fireEvent.click(screen.getByLabelText("Dismiss"));
    expect(onDismiss).toHaveBeenCalled();
  });
});
