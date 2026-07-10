import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);
const red = (s: ReturnType<typeof initialState>, p: unknown) =>
  reduce(s, { type: "frame", frame: frame(p) });

describe("approval attribution + waiting-approval card state", () => {
  const approvalFrame: Inbound = {
    v: 1, session_id: "s", id: "c9", kind: "approval_request",
    summary: "run: echo hi",
    origin: { delegation_id: "c1", subagent: "explore", depth: 1 },
  } as Inbound;

  it("stores origin on pendingApproval", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: approvalFrame });
    expect(s.pendingApproval?.origin?.subagent).toBe("explore");
  });

  it("marks the dispatch card waiting-approval and clears on answer", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = reduce(s, { type: "frame", frame: approvalFrame });
    const card = () =>
      (s.items.find((i) => i.kind === "tool" && i.id === "c1") as any).subagent;
    expect(card().waitingApproval).toBe(true);
    s = reduce(s, { type: "approval_sent" });
    expect(card().waitingApproval).toBe(false);
    expect(s.pendingApproval).toBeNull();
  });

  it("subagent_end also clears waiting-approval", () => {
    let s = initialState([]);
    s = red(s, { type: "tool_start", id: "c1", name: "dispatch_agent", args: {} });
    s = red(s, { type: "subagent_start", id: "c1", subagent_type: "explore" });
    s = reduce(s, { type: "frame", frame: approvalFrame });
    s = red(s, { type: "subagent_end", id: "c1", outcome: "completed" });
    const card = (s.items.find((i) => i.kind === "tool" && i.id === "c1") as any).subagent;
    expect(card.waitingApproval).toBe(false);
  });

  it("parent approval (no origin) touches no card", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: {
      v: 1, session_id: "s", id: "c9", kind: "approval_request", summary: "run: rm x",
    } as Inbound });
    expect(s.pendingApproval?.origin).toBeUndefined();
  });
});
