import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TopBar } from "./TopBar";

const base = { projectLabel: "studio-x", online: true, status: "open" as const,
  theme: "light" as const, onToggleTheme: () => {}, onSignOut: () => {} };

describe("TopBar", () => {
  it("shows the project label and online state", () => {
    render(<TopBar {...base} />);
    expect(screen.getByText("studio-x")).toBeInTheDocument();
  });
  it("opens settings and signs out", () => {
    const onOpenSettings = vi.fn(); const onSignOut = vi.fn();
    render(<TopBar {...base} onOpenSettings={onOpenSettings} onSignOut={onSignOut} />);
    fireEvent.click(screen.getByLabelText("settings"));
    fireEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(onOpenSettings).toHaveBeenCalled();
    expect(onSignOut).toHaveBeenCalled();
  });
  it("shows the workspace toggle only when asked", () => {
    const onToggleWorkspace = vi.fn();
    const { rerender } = render(<TopBar {...base} />);
    expect(screen.queryByLabelText("toggle workspace")).toBeNull();
    rerender(<TopBar {...base} showWorkspaceToggle onToggleWorkspace={onToggleWorkspace} />);
    fireEvent.click(screen.getByLabelText("toggle workspace"));
    expect(onToggleWorkspace).toHaveBeenCalled();
  });
});
