import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "../src/App";

beforeEach(() => localStorage.clear());

describe("App", () => {
  it("renders the pairing screen when no session is stored", () => {
    render(<App />);
    expect(screen.getByText(/pair with your agent/i)).toBeInTheDocument();
  });
});
