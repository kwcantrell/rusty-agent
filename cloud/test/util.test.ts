import { describe, it, expect } from "vitest";
import { sha256hex, newToken, newPairingCode } from "../src/util";

describe("util", () => {
  it("hashes deterministically", async () => {
    expect(await sha256hex("abc")).toEqual(await sha256hex("abc"));
    expect(await sha256hex("abc")).not.toEqual(await sha256hex("abd"));
  });
  it("makes a 6-digit pairing code", () => {
    expect(newPairingCode()).toMatch(/^\d{6}$/);
  });
  it("makes distinct tokens", () => {
    expect(newToken()).not.toEqual(newToken());
  });
});
