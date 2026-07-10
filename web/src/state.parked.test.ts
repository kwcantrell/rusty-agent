import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const runs = [{ session_id: "100-aaaaaaaa", workspace: "/w", created_ms: 5, asks: 1 }];
const parkedFrame = { v: 1, session_id: "s", kind: "parked_runs", runs } as Inbound;

describe("parked runs + retraction + resumed", () => {
  it("stores the parked_runs snapshot", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: parkedFrame });
    expect(s.parkedRuns).toEqual(runs);
  });

  it("approval_resolved clears a matching pending approval and card flags", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run x",
    } as Inbound });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "approval_resolved", id: "c9",
    } as Inbound });
    expect(s.pendingApproval).toBeNull();
  });

  it("approval_resolved with a different id leaves the prompt alone", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run x",
    } as Inbound });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "approval_resolved", id: "c8",
    } as Inbound });
    expect(s.pendingApproval?.id).toBe("c9");
  });

  it("resumed drops the banner row", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: parkedFrame });
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", kind: "resumed", resumed_session_id: "100-aaaaaaaa",
    } as Inbound });
    expect(s.parkedRuns).toEqual([]);
  });
});
