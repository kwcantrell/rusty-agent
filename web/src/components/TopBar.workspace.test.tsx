import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TopBar } from "./TopBar";

const invokeMock = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invokeMock(...a) }));

describe("TopBar workspace control", () => {
  it("shows the workspace and calls pick_workspace on click in Tauri mode", async () => {
    invokeMock.mockReset().mockResolvedValue("/home/u/new");
    const onChanged = vi.fn();
    render(
      <TopBar
        projectLabel="proj"
        online={true}
        status="open"
        theme="dark"
        onToggleTheme={() => {}}
        onSignOut={() => {}}
        tauriWorkspace="/home/u/proj"
        onWorkspaceChanged={onChanged}
      />,
    );
    expect(screen.getByText("/home/u/proj")).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: /change/i }));
    expect(invokeMock).toHaveBeenCalledWith("pick_workspace");
  });
});
