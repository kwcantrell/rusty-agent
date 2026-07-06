import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const tauriMock = vi.hoisted(() => ({ value: true }));
vi.mock("../transport", () => ({ isTauri: () => tauriMock.value }));

import { RightPaneTabs } from "./RightPaneTabs";

describe("RightPaneTabs", () => {
  beforeEach(() => { tauriMock.value = true; });

  it("renders all five tabs under Tauri", () => {
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    for (const name of ["Workspace", "Context", "Design", "Architecture", "Config"]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("hides Architecture and Config outside Tauri", () => {
    tauriMock.value = false;
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    expect(screen.getByRole("tab", { name: "Design" })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Architecture" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Config" })).not.toBeInTheDocument();
  });

  it("selects Config on click", () => {
    const picked: string[] = [];
    render(<RightPaneTabs rightTab="workspace" setRightTab={(t) => picked.push(t)} />);
    fireEvent.click(screen.getByRole("tab", { name: "Config" }));
    expect(picked).toEqual(["config"]);
    expect(screen.getByRole("tab", { name: "Workspace" })).toHaveAttribute("aria-selected", "true");
  });
});
