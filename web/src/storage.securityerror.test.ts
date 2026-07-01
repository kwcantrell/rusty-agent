import { describe, it, expect, afterEach, vi } from "vitest";
import { loadRightTab, loadTheme } from "./storage";

describe("storage load helpers under a blocked localStorage", () => {
  afterEach(() => vi.restoreAllMocks());

  it("loadRightTab falls back to 'workspace' when getItem throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new DOMException("blocked", "SecurityError"); });
    expect(loadRightTab()).toBe("workspace");
  });

  it("loadTheme falls back to null when getItem throws", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new DOMException("blocked", "SecurityError"); });
    expect(loadTheme()).toBeNull();
  });
});
