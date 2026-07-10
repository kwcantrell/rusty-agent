import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { ContextExplorer } from "./ContextExplorer";

vi.mock("./api", () => ({
  getContext: vi.fn().mockResolvedValue({
    turn: 1, model_limit: 1000, est_total: 60,
    segments: [{ category: "system", est_tokens: 60, items: ["You are..."], count: 1 }],
  }),
}));

describe("ContextExplorer", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the breakdown total after fetching a snapshot", async () => {
    render(<ContextExplorer realTotal={100} refreshKey={0} skills={[]} />);
    expect(await screen.findByText(/100/)).toBeInTheDocument();
    expect(await screen.findByText(/system/i)).toBeInTheDocument();
  });

  it("clicking the synthetic unattributed slice shows the gap panel", async () => {
    render(<ContextExplorer realTotal={100} refreshKey={0} skills={[]} />);
    // realTotal (100) > est_total (60) → an "unattributed" legend button appears.
    const btn = await screen.findByRole("button", { name: /unattributed/i });
    fireEvent.click(btn);
    expect(await screen.findByText(/Gap between server total and estimated sum/)).toBeInTheDocument();
    expect(screen.getByText(/40 tokens unaccounted/)).toBeInTheDocument();
  });

  it("clicking a segment legend button shows its items; clicking again collapses", async () => {
    render(<ContextExplorer realTotal={null} refreshKey={0} skills={[]} />);
    // Wait for snapshot to load — the legend button for "system" will appear
    const btn = await screen.findByRole("button", { name: /system/i });
    // Items must not be visible before the first click
    expect(screen.queryByText(/You are\.\.\./)).toBeNull();
    // Open the drill-in panel
    fireEvent.click(btn);
    expect(await screen.findByText(/You are\.\.\./)).toBeInTheDocument();
    // Collapse by clicking the same button again
    fireEvent.click(btn);
    await waitFor(() => expect(screen.queryByText(/You are\.\.\./)).toBeNull());
  });
});
