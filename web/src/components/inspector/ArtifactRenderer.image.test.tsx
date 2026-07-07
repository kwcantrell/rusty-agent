import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactRenderer } from "./ArtifactRenderer";
import type { Display } from "../../wire";

const img = (data: string, mime = "image/png"): Display =>
  ({ Image: { mime, data } }) as Display;

describe("ArtifactRenderer Image", () => {
  it("renders data: URIs verbatim", () => {
    render(<ArtifactRenderer display={img("data:image/png;base64,AAAA")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "data:image/png;base64,AAAA");
  });

  it("wraps raw base64 into a data URI", () => {
    render(<ArtifactRenderer display={img("AAAA")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "data:image/png;base64,AAAA");
  });

  it("renders localhost http images", () => {
    render(<ArtifactRenderer display={img("http://localhost:5173/chart.png")} />);
    expect(screen.getByRole("img")).toHaveAttribute("src", "http://localhost:5173/chart.png");
  });

  it("blocks remote http(s) images with a notice instead of an img", () => {
    render(<ArtifactRenderer display={img("https://evil.com/pixel.gif")} />);
    expect(screen.queryByRole("img")).not.toBeInTheDocument();
    expect(screen.getByText(/blocked remote image/i)).toBeInTheDocument();
  });
});
