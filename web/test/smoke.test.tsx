import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "../src/App";

beforeEach(() => localStorage.clear());

describe("App", () => {
  it("shows the desktop-app notice when not running under Tauri", () => {
    render(<App />);
    expect(screen.getByText(/desktop app/i)).toBeInTheDocument();
  });
});
