import { describe, it, expect, beforeEach } from "vitest";
import { loadRightTab, saveRightTab } from "./storage";

describe("right tab persistence", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to workspace", () => {
    expect(loadRightTab()).toBe("workspace");
  });

  it("round-trips design", () => {
    saveRightTab("design");
    expect(loadRightTab()).toBe("design");
  });

  it("round-trips context", () => {
    saveRightTab("context");
    expect(loadRightTab()).toBe("context");
  });

  it("falls back to workspace on a stale stored value", () => {
    localStorage.setItem("rightTab", "garbage");
    expect(loadRightTab()).toBe("workspace");
  });
});
