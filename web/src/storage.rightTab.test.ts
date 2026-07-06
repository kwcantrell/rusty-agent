import { describe, it, expect, beforeEach } from "vitest";
import { loadRightTab, saveRightTab } from "./storage";

describe("right tab persistence", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to workspace", () => {
    expect(loadRightTab(true)).toBe("workspace");
  });

  it("round-trips design", () => {
    saveRightTab("design");
    expect(loadRightTab(false)).toBe("design");
  });

  it("round-trips context", () => {
    saveRightTab("context");
    expect(loadRightTab(false)).toBe("context");
  });

  it("round-trips architecture and config under Tauri", () => {
    saveRightTab("architecture");
    expect(loadRightTab(true)).toBe("architecture");
    saveRightTab("config");
    expect(loadRightTab(true)).toBe("config");
  });

  it("falls back to workspace for Tauri-only tabs outside Tauri", () => {
    saveRightTab("architecture");
    expect(loadRightTab(false)).toBe("workspace");
    saveRightTab("config");
    expect(loadRightTab(false)).toBe("workspace");
  });

  it("falls back to workspace on a stale stored value", () => {
    localStorage.setItem("rightTab", "garbage");
    expect(loadRightTab(true)).toBe("workspace");
  });
});
