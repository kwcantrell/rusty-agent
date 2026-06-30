import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemorySection } from "./MemorySection";

vi.mock("./api", () => ({
  listMemories: vi.fn().mockResolvedValue([
    { id: "m1", text: "cargo not on PATH", tags: ["setup"], scope_kind: "global", updated_at: 1 },
  ]),
  deleteMemory: vi.fn().mockResolvedValue(true),
  updateMemory: vi.fn(),
  recallPreview: vi.fn().mockResolvedValue([]),
}));
import { deleteMemory } from "./api";

describe("MemorySection", () => {
  beforeEach(() => vi.clearAllMocks());
  it("lists stored memories and deletes one", async () => {
    render(<MemorySection recalled={[]} />);
    expect(await screen.findByText(/cargo not on PATH/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /delete m1/i }));
    await waitFor(() => expect(deleteMemory).toHaveBeenCalledWith("m1"));
  });
});
