import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactRenderer } from "../src/components/inspector/ArtifactRenderer";

describe("ArtifactRenderer", () => {
  it("renders a Markdown artifact", () => {
    render(<ArtifactRenderer display={{ Markdown: { text: "# Title here" } }} />);
    expect(screen.getByText("Title here")).toBeInTheDocument();
  });
  it("renders a Code artifact with filename header", () => {
    const { container } = render(<ArtifactRenderer display={{ Code: { lang: "rust", filename: "a.rs", text: "fn x(){}" } }} />);
    expect(screen.getByText("a.rs")).toBeInTheDocument();
    // rehype-highlight splits code across <span>s; assert on combined text content.
    expect(container.querySelector("code")?.textContent).toContain("fn x");
  });
  it("renders a Table artifact", () => {
    render(<ArtifactRenderer display={{ Table: { columns: ["A", "B"], rows: [["1", "2"]] } }} />);
    expect(screen.getByText("A")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });
  it("renders a Diff artifact", () => {
    render(<ArtifactRenderer display={{ Diff: { path: "a.txt", before: "foo\n", after: "bar\n" } }} />);
    expect(screen.getByText("a.txt")).toBeInTheDocument();
  });
});
