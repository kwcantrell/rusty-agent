import { describe, it, expect } from "vitest";
import { artifactsFrom, type Item } from "../src/state";

describe("artifactsFrom", () => {
  it("extracts tool items that carry a display, titled by display.title or tool name", () => {
    const items: Item[] = [
      { kind: "user", text: "hi" },
      { kind: "tool", name: "render", args: {}, status: "done",
        display: { Markdown: { text: "# Plan", title: "Plan" } } },
      { kind: "tool", name: "edit_file", args: {}, status: "done",
        display: { Diff: { path: "a.rs", before: "x", after: "y" } } },
      { kind: "tool", name: "noop", args: {}, status: "running" },
    ];
    const arts = artifactsFrom(items);
    expect(arts.map((a) => a.title)).toEqual(["Plan", "edit_file"]);
    expect(arts.map((a) => a.key)).toEqual(["art-1", "art-2"]);
  });
});
