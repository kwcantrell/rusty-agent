import { describe, expect, it } from "vitest";
import { routeFor } from "./router";
import { formatStats } from "./stats";

describe("routes", () => {
  it("home", () => {
    const p = routeFor("/")!;
    expect(p.title).toBe("Acme Dashboard Home");
    expect(p.body).toContain("Welcome to Acme");
  });
  it("pricing", () => {
    const p = routeFor("/pricing")!;
    expect(p.title).toBe("Plans & Pricing");
    expect(p.body).toContain("Starter: $9/mo");
  });
  it("about", () => {
    const p = routeFor("/about")!;
    expect(p.title).toBe("About Acme");
    expect(p.body).toContain("Founded 2019");
  });
  it("stats page", () => {
    const p = routeFor("/stats")!;
    expect(p.title).toBe("Usage Statistics");
    expect(p.body).toContain("Daily Active Users");
  });
  it("unknown routes are null", () => {
    expect(routeFor("/nope")).toBeNull();
  });
});

describe("stats view", () => {
  it("maps every field", () => {
    const v = formatStats({ dau: 88, p95_ms: 5, uptime_pct: 100 });
    expect(v.dailyActive).toBe(88);
    expect(v.latencyP95).toBe("5 ms");
    expect(v.uptime).toBe("100%");
  });
});
