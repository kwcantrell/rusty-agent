import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { SkillSection } from "./SkillSection";

vi.mock("./api", () => ({
  getSkill: vi.fn().mockResolvedValue({ name: "greeter", description: "says hi", body: "Say hi.", files: [] }),
  saveSkill: vi.fn().mockResolvedValue(undefined),
}));
import { getSkill, saveSkill } from "./api";

describe("SkillSection", () => {
  beforeEach(() => vi.clearAllMocks());

  it("opens a skill body on click", async () => {
    render(<SkillSection skills={[{ name: "greeter", description: "says hi" }]} />);
    fireEvent.click(screen.getByRole("button", { name: /greeter/i }));
    await waitFor(() => expect(getSkill).toHaveBeenCalledWith("greeter"));
    expect(await screen.findByDisplayValue(/Say hi\./)).toBeInTheDocument();
  });

  it("shows inline error when saveSkill rejects (FIX D)", async () => {
    vi.mocked(saveSkill).mockRejectedValueOnce(new Error("skill_save failure"));
    render(<SkillSection skills={[{ name: "greeter", description: "says hi" }]} />);
    // Open the skill
    fireEvent.click(screen.getByRole("button", { name: /greeter/i }));
    await screen.findByDisplayValue(/Say hi\./);
    // Attempt to save
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));
    await waitFor(() => expect(screen.getByText(/skill_save failure/)).toBeInTheDocument());
  });
});
