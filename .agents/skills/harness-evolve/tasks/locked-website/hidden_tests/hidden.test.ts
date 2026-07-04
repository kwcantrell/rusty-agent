import { describe, expect, it } from "vitest";
import { routeFor } from "./router";
import { formatBook } from "./catalog";

describe("routes", () => {
  it("home", () => {
    const p = routeFor("/")!;
    expect(p.title).toBe("Orbit Books Home");
    expect(p.body).toContain("Welcome to Orbit Books");
  });
  it("catalog", () => {
    const p = routeFor("/catalog")!;
    expect(p.title).toBe("Book Catalog");
    expect(p.body).toContain("Browse our shelves");
  });
  it("contact", () => {
    const p = routeFor("/contact")!;
    expect(p.title).toBe("Contact Orbit");
    expect(p.body).toContain("orbit@example.com");
  });
  it("unknown routes are null", () => {
    expect(routeFor("/nope")).toBeNull();
  });
});

describe("formatBook", () => {
  it("maps an in-stock book", () => {
    const v = formatBook({ title: "Dune", price_cents: 1999, in_stock: true });
    expect(v.title).toBe("Dune");
    expect(v.price).toBe("$19.99");
    expect(v.availability).toBe("In stock");
  });
  it("maps an out-of-stock book with trailing zeros", () => {
    const v = formatBook({ title: "Foundation", price_cents: 500, in_stock: false });
    expect(v.price).toBe("$5.00");
    expect(v.availability).toBe("Out of stock");
  });
});
