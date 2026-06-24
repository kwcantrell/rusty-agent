import { describe, it, expect, beforeEach } from "vitest";
import { loadWorkspaceView, saveWorkspaceView } from "./storage";

describe("workspace view persistence", () => {
  beforeEach(() => localStorage.clear());
  it("defaults to preview/desktop", () => {
    expect(loadWorkspaceView()).toEqual({ mode: "preview", viewport: "desktop" });
  });
  it("round-trips a saved view", () => {
    saveWorkspaceView({ mode: "code", viewport: "mobile" });
    expect(loadWorkspaceView()).toEqual({ mode: "code", viewport: "mobile" });
  });
  it("falls back to defaults on garbage", () => {
    localStorage.setItem("agent.workspaceView", "not json");
    expect(loadWorkspaceView()).toEqual({ mode: "preview", viewport: "desktop" });
  });
});
