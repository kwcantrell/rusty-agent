import { describe, it, expect } from "vitest";
import { resolveInitialTheme } from "../src/theme";

describe("resolveInitialTheme", () => {
  it("prefers a stored choice", () => {
    expect(resolveInitialTheme("light", true)).toBe("light");
    expect(resolveInitialTheme("dark", false)).toBe("dark");
  });
  it("falls back to system preference", () => {
    expect(resolveInitialTheme(null, true)).toBe("dark");
    expect(resolveInitialTheme(null, false)).toBe("light");
  });
});
