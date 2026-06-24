import { describe, it, expect, beforeEach } from "vitest";
import { loadToken, saveSession, clearSession, loadUserMsgs, appendUserMsg, loadSessionId, loadDashExpanded, saveDashExpanded } from "../src/storage";

beforeEach(() => localStorage.clear());

describe("storage", () => {
  it("saves and loads a session, and clears it", () => {
    saveSession("sess-1", "tok-abc");
    expect(loadToken()).toBe("tok-abc");
    expect(loadSessionId()).toBe("sess-1");
    clearSession();
    expect(loadToken()).toBeNull();
    expect(loadSessionId()).toBeNull();
  });
  it("appends and loads per-session user messages", () => {
    appendUserMsg("sess-1", "q1");
    appendUserMsg("sess-1", "q2");
    appendUserMsg("sess-2", "other");
    expect(loadUserMsgs("sess-1")).toEqual(["q1", "q2"]);
    expect(loadUserMsgs("sess-2")).toEqual(["other"]);
    expect(loadUserMsgs("nope")).toEqual([]);
  });
});

describe("context dashboard persistence", () => {
  it("defaults to collapsed and round-trips the expanded flag", () => {
    localStorage.clear();
    expect(loadDashExpanded()).toBe(false);
    saveDashExpanded(true);
    expect(loadDashExpanded()).toBe(true);
    saveDashExpanded(false);
    expect(loadDashExpanded()).toBe(false);
  });
});
