import { describe, it, expect } from "vitest";
import { animatedItemsFrom, turnGroupsFrom, type AnimatedItem } from "../src/state";
import type { Item } from "../src/state";

function makeItem(kind: string, props: Record<string, unknown>): Item {
  return { kind, ...props } as Item;
}

describe("animatedItemsFrom", () => {
  it("marks items with timestamps and streaming state", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("user", { text: "hello" }),
      makeItem("assistant", { text: "hi there" }),
      makeItem("tool", { name: "read_file", args: {}, status: "running" }),
      makeItem("assistant", { text: "the answer", done: "stop" }),
    ];
    const animated = animatedItemsFrom(items, now);
    expect(animated).toHaveLength(4);
    // First item gets ts=now, subsequent items get increasing ts
    expect(animated[0].ts).toBe(now);
    expect(animated[0].streaming).toBe(false); // user items are not streaming
    expect(animated[0].progress).toBe(1); // not streaming → fully rendered
    expect(animated[3].streaming).toBe(false); // done items are not streaming
    expect(animated[3].progress).toBe(1);
  });

  it("marks tool items as streaming while running, not streaming when done", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("tool", { name: "x", args: {}, status: "running" }),
      makeItem("tool", { name: "y", args: {}, status: "done", content: "ok" }),
    ];
    const animated = animatedItemsFrom(items, now);
    expect(animated[0].streaming).toBe(true);
    expect(animated[0].progress).toBe(0);
    expect(animated[1].streaming).toBe(false);
    expect(animated[1].progress).toBe(1);
  });

  it("marks reasoning items as streaming", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("reasoning", { text: "thinking..." }),
    ];
    const animated = animatedItemsFrom(items, now);
    expect(animated[0].streaming).toBe(true);
  });

  it("assigns increasing timestamps to items", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("user", { text: "a" }),
      makeItem("assistant", { text: "b" }),
      makeItem("assistant", { text: "c", done: "stop" }),
      makeItem("user", { text: "d" }),
    ];
    const animated = animatedItemsFrom(items, now);
    expect(animated[0].ts).toBe(now);
    expect(animated[1].ts).toBeGreaterThan(now);
    expect(animated[2].ts).toBeGreaterThanOrEqual(animated[1].ts);
    expect(animated[3].ts).toBeGreaterThanOrEqual(animated[2].ts);
  });
});

describe("turnGroupsFrom", () => {
  it("groups items between done signals", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("user", { text: "hello" }),
      makeItem("assistant", { text: "hi", done: "stop" }),
      makeItem("user", { text: "again" }),
      makeItem("assistant", { text: "hey", done: "stop" }),
    ];
    const animated = animatedItemsFrom(items, now);
    const groups = turnGroupsFrom(animated);
    expect(groups).toHaveLength(2);
    // First turn: user "hello" + assistant "hi"
    expect(groups[0].items).toHaveLength(2);
    expect(groups[0].items[0]).toMatchObject({ kind: "user", text: "hello" });
    expect(groups[0].items[1]).toMatchObject({ kind: "assistant", done: "stop" });
    // Second turn: user "again" + assistant "hey"
    expect(groups[1].items).toHaveLength(2);
    expect(groups[1].items[0]).toMatchObject({ kind: "user", text: "again" });
  });

  it("computes turn duration from timestamps", () => {
    const base = Date.now();
    const items: AnimatedItem[] = [
      { kind: "user", text: "q", ts: base, streaming: false, progress: 1 } as AnimatedItem,
      { kind: "assistant", text: "a", ts: base + 100, streaming: false, progress: 1 } as AnimatedItem,
      { kind: "assistant", text: "a", done: "stop", ts: base + 200, streaming: false, progress: 1 } as AnimatedItem,
    ];
    const groups = turnGroupsFrom(items);
    expect(groups[0].startTs).toBe(base);
    expect(groups[0].endTs).toBe(base + 200);
    expect(groups[0].duration).toBe(200);
  });

  it("handles tool items within a turn", () => {
    const now = Date.now();
    const items: Item[] = [
      makeItem("user", { text: "run x" }),
      makeItem("tool", { name: "run", args: {}, status: "running" }),
      makeItem("tool", { name: "run", args: {}, status: "done", content: "ok" }),
      makeItem("assistant", { text: "done", done: "stop" }),
    ];
    const animated = animatedItemsFrom(items, now);
    const groups = turnGroupsFrom(animated);
    expect(groups[0].items).toHaveLength(4);
    expect(groups[0].items[1]).toMatchObject({ kind: "tool", name: "run" });
  });
});
