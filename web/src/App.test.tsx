import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App shell", () => {
  beforeEach(() => localStorage.clear());
  it("renders the desktop-app notice (not the two-pane shell) when not under Tauri", () => {
    render(<App />);
    // The connected shell shows a "sign out" control; the notice must not.
    expect(screen.queryByText(/sign out/i)).toBeNull();
  });
});
