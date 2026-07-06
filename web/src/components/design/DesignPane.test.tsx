import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { Item } from "../../state";
import { DesignPane } from "./DesignPane";

const designItem = (html: string): Item =>
  ({ kind: "tool", name: "render", args: {}, status: "done",
     display: { Html: { html, id: "design:landing", title: "Landing" } } });

const base = { sessionId: "s1", onSend: () => {}, sendDisabled: false };

describe("DesignPane", () => {
  beforeEach(() => { localStorage.clear(); });

  it("shows an empty state with no designs", () => {
    render(<DesignPane {...base} items={[]} />);
    expect(screen.getByText(/No designs yet/)).toBeInTheDocument();
  });

  it("renders the latest design version in the canvas", () => {
    render(<DesignPane {...base} items={[designItem("<p>v1</p>"), designItem("<p>v2</p>")]} />);
    expect(screen.getByText("v2 / 2")).toBeInTheDocument();
  });

  it("has no Config or Architecture sub-tabs", () => {
    render(<DesignPane {...base} items={[]} />);
    expect(screen.queryByRole("tab", { name: "Config" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Architecture" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Canvas" })).not.toBeInTheDocument();
  });

  it("sends structured feedback and records sent pins", () => {
    const sent: string[] = [];
    render(<DesignPane {...base} items={[designItem("<p>v1</p>")]} onSend={(t) => sent.push(t)} />);
    const layer = screen.getByTestId("pin-layer");
    vi.spyOn(layer.parentElement as HTMLElement, "getBoundingClientRect").mockReturnValue({
      left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}),
    } as DOMRect);
    fireEvent.click(layer, { clientX: 50, clientY: 50 });
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "bigger" } });
    fireEvent.click(screen.getByRole("button", { name: /Send feedback/ }));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toContain("```design-feedback");
    expect(sent[0]).toContain('"design_id": "design:landing"');
    expect(screen.getAllByTestId("pin-sent")).toHaveLength(1); // retained as sent
  });
});
