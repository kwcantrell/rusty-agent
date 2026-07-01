import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemorySection } from "./MemorySection";

vi.mock("./api", () => ({
  listMemories: vi.fn().mockResolvedValue([
    { id: "m1", text: "cargo not on PATH", tags: ["setup"], scope_kind: "global", updated_at: 1 },
  ]),
  deleteMemory: vi.fn().mockResolvedValue(true),
  updateMemory: vi.fn().mockResolvedValue({}),
  recallPreview: vi.fn().mockResolvedValue([]),
}));
import { deleteMemory, recallPreview } from "./api";

describe("MemorySection", () => {
  beforeEach(() => vi.clearAllMocks());

  it("lists stored memories and deletes one", async () => {
    render(<MemorySection recalled={[]} />);
    expect(await screen.findByText(/cargo not on PATH/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /delete m1/i }));
    await waitFor(() => expect(deleteMemory).toHaveBeenCalledWith("m1"));
  });

  it("shows cosine scores when recallPreview returns scored rows (FIX C)", async () => {
    vi.mocked(recallPreview).mockResolvedValueOnce([
      { id: "r1", text: "recall row text", score: 0.84, scope_kind: "session" },
    ]);
    render(<MemorySection recalled={["plain fallback"]} lastQuery="cargo" />);
    // Score and text from ScoredRow should appear
    expect(await screen.findByText(/0\.84/)).toBeInTheDocument();
    expect(screen.getByText("recall row text")).toBeInTheDocument();
    expect(screen.getByText("[session]")).toBeInTheDocument();
    // Plain fallback list must be replaced by scored rows
    expect(screen.queryByText(/plain fallback/)).toBeNull();
  });

  it("shows inline error when deleteMemory rejects (FIX D)", async () => {
    vi.mocked(deleteMemory).mockRejectedValueOnce(new Error("scope-guard refusal"));
    render(<MemorySection recalled={[]} />);
    await screen.findByText(/cargo not on PATH/);
    fireEvent.click(screen.getByRole("button", { name: /delete m1/i }));
    await waitFor(() => expect(screen.getByText(/scope-guard refusal/)).toBeInTheDocument());
  });

  it("surfaces an error when the initial memory load fails", async () => {
    const api = await import("./api");
    (api.listMemories as unknown as { mockRejectedValueOnce: (e: Error) => void })
      .mockRejectedValueOnce(new Error("boom"));
    render(<MemorySection recalled={[]} lastQuery={null} />);
    expect(await screen.findByText(/boom/)).toBeInTheDocument();
  });
});
