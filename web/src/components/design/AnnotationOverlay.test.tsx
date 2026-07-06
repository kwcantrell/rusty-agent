import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AnnotationOverlay } from "./AnnotationOverlay";

// jsdom has no layout: give the pin layer a fake box so pct math works.
function mockBox(el: HTMLElement) {
  vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
    left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON: () => ({}),
  } as DOMRect);
}

describe("AnnotationOverlay", () => {
  it("click adds a draft pin at pct coords; comment enables send", () => {
    const onSend = vi.fn();
    render(<AnnotationOverlay sent={[]} disabled={false} onSend={onSend}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 100, clientY: 25 });
    expect(screen.getAllByTestId("pin-draft")).toHaveLength(1);

    const send = screen.getByRole("button", { name: /Send feedback/ });
    expect(send).toBeDisabled(); // empty comment
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "move this" } });
    expect(send).toBeEnabled();
    fireEvent.click(send);
    expect(onSend).toHaveBeenCalledWith([{ x_pct: 0.5, y_pct: 0.25, comment: "move this" }]);
    expect(screen.queryAllByTestId("pin-draft")).toHaveLength(0); // drafts cleared
  });

  it("deletes a draft pin", () => {
    render(<AnnotationOverlay sent={[]} disabled={false} onSend={() => {}}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 10, clientY: 10 });
    fireEvent.click(screen.getByRole("button", { name: "delete pin 1" }));
    expect(screen.queryAllByTestId("pin-draft")).toHaveLength(0);
  });

  it("renders sent pins as read-only markers", () => {
    render(<AnnotationOverlay sent={[{ x_pct: 0.1, y_pct: 0.2, comment: "done" }]} disabled={false}
      onSend={() => {}}><p>art</p></AnnotationOverlay>);
    expect(screen.getAllByTestId("pin-sent")).toHaveLength(1);
  });

  it("send stays disabled when the composer is disabled", () => {
    render(<AnnotationOverlay sent={[]} disabled={true} onSend={() => {}}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 10, clientY: 10 });
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "x" } });
    expect(screen.getByRole("button", { name: /Send feedback/ })).toBeDisabled();
  });

  it("aria-label numbering accounts for existing sent pins", () => {
    const sent = [
      { x_pct: 0.1, y_pct: 0.1, comment: "first" },
      { x_pct: 0.2, y_pct: 0.2, comment: "second" },
    ];
    render(<AnnotationOverlay sent={sent} disabled={false} onSend={() => {}}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 10, clientY: 10 });
    // draft is pin #3 because two sent pins already exist
    expect(screen.getByLabelText("pin 3 comment")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "delete pin 3" })).toBeInTheDocument();
  });
});
