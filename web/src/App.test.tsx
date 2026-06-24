import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App shell", () => {
  beforeEach(() => localStorage.clear());
  it("renders the pairing screen (not the two-pane shell) when unauthenticated", () => {
    render(<App />);
    // The authenticated shell shows a "sign out" control; unauthenticated must not.
    expect(screen.queryByText(/sign out/i)).toBeNull();
  });
});
