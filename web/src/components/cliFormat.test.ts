import { describe, expect, it } from "vitest";
import { argSummary, resultSummary, blockMeter } from "./cliFormat";

describe("argSummary", () => {
  it("returns the first string value of an object arg", () => {
    expect(argSummary({ command: "npm test", cwd: "/x" })).toBe("npm test");
  });
  it("skips non-string values to find the first string", () => {
    expect(argSummary({ lines: 20, path: "web/src/state.ts" })).toBe("web/src/state.ts");
  });
  it("accepts a bare string arg", () => {
    expect(argSummary("ls -la")).toBe("ls -la");
  });
  it("uses only the first line of a multi-line value", () => {
    expect(argSummary({ script: "line one\nline two" })).toBe("line one");
  });
  it("truncates to 60 chars with an ellipsis", () => {
    const long = "x".repeat(80);
    const out = argSummary({ v: long })!;
    expect(out.length).toBe(60);
    expect(out.endsWith("…")).toBe(true);
  });
  it("returns null for empty object, arrays, numbers, and undefined", () => {
    expect(argSummary({})).toBeNull();
    expect(argSummary(["a"])).toBeNull();
    expect(argSummary(42)).toBeNull();
    expect(argSummary(undefined)).toBeNull();
    expect(argSummary({ v: "   " })).toBeNull();
  });
});

describe("resultSummary", () => {
  it("returns the first non-empty line", () => {
    expect(resultSummary("\n\n42 passed\n", "ok")).toBe("42 passed");
  });
  it("appends a line count when multi-line", () => {
    expect(resultSummary("a\nb\nc", "ok")).toBe("a (+2 lines)");
  });
  it("truncates the first line to 80 chars with an ellipsis", () => {
    const out = resultSummary("y".repeat(100), "ok");
    expect(out.length).toBe(80);
    expect(out.endsWith("…")).toBe(true);
  });
  it("says done for empty ok content and error for empty failed content", () => {
    expect(resultSummary("", "ok")).toBe("done");
    expect(resultSummary(undefined, "ok")).toBe("done");
    expect(resultSummary("  \n ", "error")).toBe("error");
  });
});

describe("blockMeter", () => {
  it("renders 10 cells, filled by tens", () => {
    expect(blockMeter(0)).toBe("░░░░░░░░░░");
    expect(blockMeter(60)).toBe("▂▂▂▂▂▂░░░░");
    expect(blockMeter(100)).toBe("▂▂▂▂▂▂▂▂▂▂");
  });
  it("clamps out-of-range values", () => {
    expect(blockMeter(-5)).toBe("░░░░░░░░░░");
    expect(blockMeter(140)).toBe("▂▂▂▂▂▂▂▂▂▂");
  });
});
