import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { TopBar } from "./TopBar";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const base = {
  projectLabel: "proj",
  online: true,
  status: "open" as const,
  theme: "dark" as const,
  onToggleTheme: () => {},
  onSignOut: () => {},
};

describe("TopBar llama health indicator", () => {
  it("shows a ready indicator with the model name when llamaOk is true", () => {
    render(<TopBar {...base} llamaOk={true} llamaModel="qwen3.6-35b-a3b" />);
    expect(screen.getByTitle(/qwen3\.6-35b-a3b/)).toBeTruthy();
  });

  it("shows an offline indicator when llamaOk is false", () => {
    render(<TopBar {...base} llamaOk={false} />);
    expect(screen.getByTitle(/offline/i)).toBeTruthy();
  });

  it("renders no indicator when llamaOk is undefined (browser mode)", () => {
    render(<TopBar {...base} />);
    expect(screen.queryByTitle(/llama-server/i)).toBeNull();
  });
});
