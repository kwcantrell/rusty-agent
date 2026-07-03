import { describe, expect, it } from "vitest";
import { formatStats } from "./stats";

describe("formatStats", () => {
  it("maps raw stats to the view model", () => {
    const v = formatStats({ dau: 1200, p95_ms: 142, uptime_pct: 99.95 });
    expect(v.dailyActive).toBe(1200);
    expect(v.latencyP95).toBe("142 ms");
    expect(v.uptime).toBe("99.95%");
  });
});
