import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { UrlArtifact } from "./UrlArtifact";

describe("UrlArtifact", () => {
  it("renders a live iframe for a localhost url", () => {
    render(<UrlArtifact url="http://localhost:5173/app" />);
    const frame = screen.getByTitle("live preview");
    expect(frame).toHaveAttribute("src", "http://localhost:5173/app");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts allow-same-origin");
  });

  it("blocks non-localhost urls with a notice instead of an iframe", () => {
    render(<UrlArtifact url="http://evil.com/" />);
    expect(screen.queryByTitle("live preview")).not.toBeInTheDocument();
    expect(screen.getByText(/only localhost/i)).toBeInTheDocument();
  });
});
