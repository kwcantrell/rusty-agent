import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { ContextExplorer } from "./ContextExplorer";

vi.mock("./api", () => ({
  getContext: vi.fn().mockResolvedValue({
    turn: 1, model_limit: 1000, est_total: 60,
    segments: [{ category: "system", est_tokens: 60, items: ["You are..."], count: 1 }],
  }),
  listMemories: vi.fn().mockResolvedValue([]),
  recallPreview: vi.fn().mockResolvedValue([]),
}));

describe("ContextExplorer", () => {
  beforeEach(() => vi.clearAllMocks());
  it("renders the breakdown total after fetching a snapshot", async () => {
    render(<ContextExplorer realTotal={100} refreshKey={0} />);
    expect(await screen.findByText(/100/)).toBeInTheDocument();
    expect(await screen.findByText(/system/i)).toBeInTheDocument();
  });
});
