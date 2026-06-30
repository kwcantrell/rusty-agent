import { describe, it, expect } from "vitest";
import { computeBreakdown } from "./breakdown";
import type { ContextSnapshot } from "./types";
import { reduce, initialState } from "../state";

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

describe("server_usage → unattributed integration (FIX A)", () => {
  it("server_usage event populates state.serverUsage.promptTokens and computeBreakdown yields unattributed slice", () => {
    // Feed a server_usage WireEvent through the state reducer.
    const s = reduce(initialState([]), {
      type: "frame",
      frame: { v: 1, session_id: "s1", kind: "event", payload: { type: "server_usage", prompt_tokens: 100, completion_tokens: 20, turn: 1 } },
    });
    expect(s.serverUsage?.promptTokens).toBe(100);
    // snap.est_total = 60; realTotal = 100 → unattributed = 40 tokens
    const b = computeBreakdown(snap, s.serverUsage!.promptTokens);
    const un = b.slices.find((sl) => sl.category === "unattributed");
    expect(un).toBeDefined();
    expect(un!.tokens).toBe(40);
  });
});
