import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BusyLine, busyVerb } from "./BusyLine";

describe("busyVerb", () => {
  it("cycles deterministically by turn", () => {
    expect(busyVerb(0)).toBe("Thinking");
    expect(busyVerb(1)).not.toBe(busyVerb(0));
    expect(busyVerb(6)).toBe(busyVerb(0));
  });
});

describe("BusyLine", () => {
  it("renders the spinner glyph, verb, and a seconds counter", () => {
    render(<BusyLine turn={0} />);
    expect(screen.getByText("✳")).toBeInTheDocument();
    expect(screen.getByText(/Thinking… \(0s\)/)).toBeInTheDocument();
  });
});
