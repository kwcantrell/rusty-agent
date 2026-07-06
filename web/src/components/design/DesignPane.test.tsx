import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import type { Item } from "../../state";

// ── dev-server mocks (vi.hoisted so factory can reference them without TDZ) ──
const detectDevScripts = vi.hoisted(() => vi.fn());
const startDevServer   = vi.hoisted(() => vi.fn());
const stopDevServer    = vi.hoisted(() => vi.fn());
vi.mock("./devServer", () => ({ detectDevScripts, startDevServer, stopDevServer }));
vi.mock("../../transport", async (orig) => ({ ...(await orig()), isTauri: () => true }));

import { DesignPane } from "./DesignPane";

// Safe default: 0 candidates → launcher renders nothing → existing tests unaffected.
beforeEach(() => { detectDevScripts.mockResolvedValue([]); });

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

  it("previews a manually entered localhost url on the canvas", () => {
    render(<DesignPane {...base} items={[]} />);
    fireEvent.change(screen.getByLabelText("preview url"), {
      target: { value: "http://localhost:5173" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.getByTitle("live preview")).toHaveAttribute("src", "http://localhost:5173");
  });

  it("rejects a non-localhost manual url with an inline error", () => {
    render(<DesignPane {...base} items={[]} />);
    fireEvent.change(screen.getByLabelText("preview url"), {
      target: { value: "http://evil.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.queryByTitle("live preview")).not.toBeInTheDocument();
    expect(screen.getByText(/Only localhost URLs/)).toBeInTheDocument();
  });
});

// ── Dev-server launcher tests ─────────────────────────────────────────────────
const cand = { dir: "/w/web", script: "dev", package_manager: "pnpm", label: "web — dev" };

describe("DesignPane dev-server launcher", () => {
  beforeEach(() => {
    detectDevScripts.mockReset(); startDevServer.mockReset(); stopDevServer.mockReset();
    detectDevScripts.mockResolvedValue([cand]);
  });

  it("starting a detected server renders it in the canvas", async () => {
    startDevServer.mockResolvedValue({ url: "http://localhost:5173/", candidate: cand });
    render(<DesignPane items={[]} sessionId="s1" onSend={() => {}} sendDisabled={false} />);

    const btn = await screen.findByRole("button", { name: /start dev server/i });
    fireEvent.click(btn);

    await waitFor(() => expect(startDevServer).toHaveBeenCalledWith(cand));
    // The live-preview iframe now exists (guard lets localhost through).
    await waitFor(() =>
      expect(screen.getByTitle(/live preview/i)).toBeInTheDocument());
    // Stop control appears once running.
    expect(screen.getByRole("button", { name: /stop/i })).toBeInTheDocument();
  });
});
