import { describe, it, expect } from "vitest";
import { isLocalUrl, isMixedContent } from "./urlGuard";

describe("isLocalUrl", () => {
  it("accepts http(s) loopback hosts on any port", () => {
    for (const u of ["http://localhost:5173", "https://localhost/", "http://127.0.0.1:3000/x?y=1",
      "http://[::1]:8080/x"]) {
      expect(isLocalUrl(u), u).toBe(true);
    }
  });
  it("rejects everything else, failing closed on garbage", () => {
    for (const u of ["http://evil.com", "http://localhost.evil.com:5173", "ftp://localhost/",
      "localhost:5173", "not a url", ""]) {
      expect(isLocalUrl(u), u).toBe(false);
    }
  });
});

describe("isMixedContent", () => {
  it("flags http targets only when the page itself is https", () => {
    expect(isMixedContent("http://localhost:5173", "https:")).toBe(true);
    expect(isMixedContent("http://localhost:5173", "http:")).toBe(false);
    expect(isMixedContent("https://localhost:5173", "https:")).toBe(false);
  });
});
