import { describe, expect, it } from "vitest";
import { initialState, reduce } from "./state";
import type { Item } from "./state";
import type { Inbound } from "./wire";

const frame = (payload: unknown): Inbound =>
  ({ v: 1, session_id: "s", kind: "event", payload } as Inbound);

const assistantText = (items: Item[]): string =>
  items
    .filter((i): i is Extract<Item, { kind: "assistant" }> => i.kind === "assistant")
    .map((i) => i.text)
    .join("");

describe("stream_retry retraction", () => {
  it("discards the trailing partial so only the re-streamed text survives", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: frame({ type: "token", text: "ab" }) });
    s = reduce(s, { type: "frame", frame: frame({ type: "token", text: "cd" }) });
    // The stream died after "abcd" (4 chars); a retry follows.
    s = reduce(s, { type: "frame",
      frame: frame({ type: "stream_retry", discarded_text_chars: 4, discarded_reasoning_chars: 0 }) });
    s = reduce(s, { type: "frame", frame: frame({ type: "token", text: "xy" }) });
    expect(assistantText(s.items)).toBe("xy");
  });

  it("trims only the discarded tail, keeping earlier text of the same item", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: frame({ type: "token", text: "hello" }) });
    // Only the last 2 chars ("lo") were part of the abandoned attempt.
    s = reduce(s, { type: "frame",
      frame: frame({ type: "stream_retry", discarded_text_chars: 2, discarded_reasoning_chars: 0 }) });
    expect(assistantText(s.items)).toBe("hel");
  });

  it("trims a trailing reasoning item and drops it when it empties", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: frame({ type: "reasoning", text: "think" }) });
    s = reduce(s, { type: "frame",
      frame: frame({ type: "stream_retry", discarded_text_chars: 0, discarded_reasoning_chars: 5 }) });
    expect(s.items.some((i) => i.kind === "reasoning")).toBe(false);
  });

  it("is a no-op when nothing was discarded", () => {
    let s = initialState([]);
    s = reduce(s, { type: "frame", frame: frame({ type: "token", text: "keep" }) });
    const before = s.items;
    s = reduce(s, { type: "frame",
      frame: frame({ type: "stream_retry", discarded_text_chars: 0, discarded_reasoning_chars: 0 }) });
    expect(s.items).toEqual(before);
    expect(assistantText(s.items)).toBe("keep");
  });
});
