import { describe, it, expect } from "vitest";
import { computeBreakdown } from "./breakdown";
import type { ContextSnapshot } from "./types";

const snap: ContextSnapshot = {
  turn: 1, model_limit: 1000, est_total: 60,
  segments: [
    { category: "system", est_tokens: 40, items: ["You are..."], count: 1 },
    { category: "messages", est_tokens: 20, items: [], count: 3 },
  ],
};

describe("computeBreakdown", () => {
  it("adds an unattributed slice when real total exceeds estimate", () => {
    const b = computeBreakdown(snap, 100);
    expect(b.total).toBe(100);
    const un = b.slices.find((s) => s.category === "unattributed");
    expect(un?.tokens).toBe(40); // 100 - 60
    expect(b.slices.find((s) => s.category === "unattributed")?.pct).toBe(40); // 40/100 = 40%
    expect(b.slices.find((s) => s.category === "system")?.pct).toBe(40); // 40/100 = 40%
  });
  it("clamps unattributed at zero when estimate exceeds real total", () => {
    const b = computeBreakdown(snap, 50);
    expect(b.slices.find((s) => s.category === "unattributed")).toBeUndefined();
    expect(b.total).toBe(60); // realTotal (50) < estTotal (60) → use estTotal as total
  });
  it("uses estimate as total when realTotal is null", () => {
    const b = computeBreakdown(snap, null);
    expect(b.total).toBe(60);
    expect(b.slices.find((s) => s.category === "unattributed")).toBeUndefined();
    expect(b.slices).toHaveLength(2);
  });
});
