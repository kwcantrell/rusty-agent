import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { DiffView } from "../src/components/DiffView";
import { TerminalBlock } from "../src/components/TerminalBlock";

describe("tool components", () => {
  it("DiffView shows added and removed lines", () => {
    render(<DiffView path="a.txt" before={"foo\nbar\n"} after={"foo\nbaz\n"} />);
    expect(screen.getByText("a.txt")).toBeInTheDocument();
    expect(screen.getByText(/-\s*bar/)).toBeInTheDocument();
    expect(screen.getByText(/\+\s*baz/)).toBeInTheDocument();
  });
  it("TerminalBlock shows the command, output, and exit code", () => {
    render(<TerminalBlock command="echo hi" stdout={"hi\n"} stderr="" exitCode={0} />);
    expect(screen.getByText(/echo hi/)).toBeInTheDocument();              // command span
    expect(screen.getByText(/hi/, { selector: "pre" })).toBeInTheDocument(); // stdout in <pre>
    expect(screen.getByText(/exit 0/)).toBeInTheDocument();
  });
});
