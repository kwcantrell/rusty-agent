import { describe, it, expect } from "vitest";
import { reduce, initialState } from "./state";
import type { Inbound } from "./wire";

const ev = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

describe("reasoning events", () => {
  it("accumulates reasoning into a reasoning item, separate from the answer", () => {
    let s = initialState([]);
    s = reduce(s, { type: "user_send", text: "hi" });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "plan " }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "reasoning", text: "more" }) });
    s = reduce(s, { type: "frame", frame: ev({ type: "token", text: "answer" }) });
    const reasoning = s.items.find((i) => i.kind === "reasoning");
    const assistant = s.items.find((i) => i.kind === "assistant");
    expect(reasoning).toMatchObject({ kind: "reasoning", text: "plan more" });
    expect(assistant).toMatchObject({ kind: "assistant", text: "answer" });
  });
});
