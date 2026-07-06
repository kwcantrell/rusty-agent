import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DesignCanvas } from "./DesignCanvas";
import type { Design } from "../../designStore";

const design = (n: number): Design => ({
  id: "design:x", title: "X",
  versions: Array.from({ length: n }, (_, i) =>
    ({ display: { Html: { html: `<p>v${i + 1}</p>`, id: "design:x" } }, renderable: true })),
});

const noPins = () => [];

describe("DesignCanvas", () => {
  it("shows the latest version by default and follows new versions", () => {
    const { rerender } = render(<DesignCanvas design={design(2)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v2 / 2")).toBeInTheDocument();
    rerender(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v3 / 3")).toBeInTheDocument();
  });

  it("steps back and shows a new-version badge instead of yanking the view", () => {
    const { rerender } = render(<DesignCanvas design={design(2)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    fireEvent.click(screen.getByRole("button", { name: "previous version" }));
    expect(screen.getByText("v1 / 2")).toBeInTheDocument();
    rerender(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v1 / 3")).toBeInTheDocument(); // view not yanked
    const badge = screen.getByRole("button", { name: /v3 available/ });
    fireEvent.click(badge);
    expect(screen.getByText("v3 / 3")).toBeInTheDocument();
  });

  it("compare mode renders the previous and current versions side by side", () => {
    render(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Compare" }));
    expect(screen.getByTestId("compare-left")).toBeInTheDocument();
    expect(screen.getByTestId("compare-right")).toBeInTheDocument();
  });

  it("compare is disabled with a single version", () => {
    render(<DesignCanvas design={design(1)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByRole("button", { name: "Compare" })).toBeDisabled();
  });

  it("marks an unsupported version", () => {
    const d = design(1);
    d.versions[0] = { display: { Frob: { x: 1 } } as never, renderable: false };
    render(<DesignCanvas design={d} sentPins={noPins} onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText(/unsupported/)).toBeInTheDocument();
  });
});
