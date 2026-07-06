import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import {
  displayDesignId, designsFrom, mergeDesigns, useDesignStore, MAX_VERSIONS,
} from "./designStore";
import { artifactsFrom, type Item } from "./state";
import type { Display } from "./wire";

const html = (id: string | undefined, body: string, title?: string): Display =>
  ({ Html: { html: body, title, id } });

const toolItem = (display: Display): Item =>
  ({ kind: "tool", name: "render", args: {}, status: "done", display });

describe("displayDesignId", () => {
  it("extracts a design: id", () => {
    expect(displayDesignId(html("design:landing", "<p/>"))).toBe("design:landing");
  });
  it("returns null for plain ids, missing ids, and id-less variants", () => {
    expect(displayDesignId(html("chart-1", "<p/>"))).toBeNull();
    expect(displayDesignId(html(undefined, "<p/>"))).toBeNull();
    expect(displayDesignId({ Text: "hi" })).toBeNull();
    expect(displayDesignId({ Diff: { path: "a", before: "", after: "" } })).toBeNull();
  });
});

describe("designsFrom", () => {
  it("groups displays by design id, in order, as immutable versions", () => {
    const items = [
      toolItem(html("design:landing", "<p>v1</p>", "Landing")),
      toolItem(html("chart-1", "<p>not a design</p>")),
      toolItem(html("design:landing", "<p>v2</p>", "Landing")),
      toolItem(html("design:nav", "<p>navA</p>", "Nav")),
    ];
    const designs = designsFrom(items);
    expect(designs.map((d) => d.id)).toEqual(["design:landing", "design:nav"]);
    expect(designs[0].versions).toHaveLength(2);
    expect((designs[0].versions[1].display as { Html: { html: string } }).Html.html).toBe("<p>v2</p>");
    expect(designs[0].title).toBe("Landing");
  });

  it("caps a design at MAX_VERSIONS, dropping oldest", () => {
    const items: Item[] = [];
    for (let i = 0; i < MAX_VERSIONS + 5; i++) items.push(toolItem(html("design:x", `<p>${i}</p>`)));
    const [d] = designsFrom(items);
    expect(d.versions).toHaveLength(MAX_VERSIONS);
    expect((d.versions[0].display as { Html: { html: string } }).Html.html).toBe("<p>5</p>");
  });

  it("falls back to the design id as title when untitled", () => {
    const [d] = designsFrom([toolItem(html("design:x", "<p/>"))]);
    expect(d.title).toBe("design:x");
  });
});

describe("artifactsFrom interception", () => {
  it("excludes design displays from workspace artifacts", () => {
    const items = [
      toolItem(html("design:landing", "<p/>")),
      toolItem(html(undefined, "<p>plain</p>", "plain")),
    ];
    const arts = artifactsFrom(items);
    expect(arts).toHaveLength(1);
    expect(arts[0].title).toBe("plain");
  });
});

describe("mergeDesigns", () => {
  it("prepends stored history and caps", () => {
    const stored = designsFrom([toolItem(html("design:x", "<p>old</p>"))]);
    const live = designsFrom([toolItem(html("design:x", "<p>new</p>"))]);
    const [d] = mergeDesigns(stored, live);
    expect(d.versions.map((v) => (v.display as { Html: { html: string } }).Html.html))
      .toEqual(["<p>old</p>", "<p>new</p>"]);
  });
  it("keeps stored-only and live-only designs", () => {
    const stored = designsFrom([toolItem(html("design:a", "<p/>"))]);
    const live = designsFrom([toolItem(html("design:b", "<p/>"))]);
    expect(mergeDesigns(stored, live).map((d) => d.id)).toEqual(["design:a", "design:b"]);
  });
  it("mergeDesigns keeps legitimately repeated identical renders (multiset dedup)", () => {
    const twice = [toolItem(html("design:x", "<p>same</p>")), toolItem(html("design:x", "<p>same</p>"))];
    const stored = designsFrom(twice);
    const [d] = mergeDesigns(stored, designsFrom(twice)); // remount case
    expect(d.versions).toHaveLength(2); // not 4, not 1
  });
});

describe("useDesignStore", () => {
  beforeEach(() => localStorage.clear());

  it("persists designs and sent pins; a remount restores them", () => {
    const items = [toolItem(html("design:x", "<p>v1</p>"))];
    const first = renderHook(({ it: i }) => useDesignStore(i, "sess-1"), { initialProps: { it: items } });
    act(() => first.result.current.recordSent("design:x", 1, [{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]));
    first.unmount();

    // fresh mount, empty items (reload wiped the reducer)
    const second = renderHook(() => useDesignStore([], "sess-1"));
    expect(second.result.current.designs).toHaveLength(1);
    expect(second.result.current.designs[0].versions).toHaveLength(1);
    expect(second.result.current.sentPins("design:x", 1)).toEqual([{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]);
  });

  it("isolates sessions", () => {
    const a = renderHook(() => useDesignStore([toolItem(html("design:x", "<p/>"))], "sess-a"));
    a.unmount();
    const b = renderHook(() => useDesignStore([], "sess-b"));
    expect(b.result.current.designs).toHaveLength(0);
  });

  it("survives a broken localStorage (SecurityError fallback)", () => {
    const orig = Storage.prototype.setItem;
    Storage.prototype.setItem = () => { throw new Error("SecurityError"); };
    try {
      const h = renderHook(() => useDesignStore([toolItem(html("design:x", "<p/>"))], "sess-1"));
      expect(h.result.current.designs).toHaveLength(1); // in-memory still works
    } finally { Storage.prototype.setItem = orig; }
  });

  it("does not duplicate versions when remounted mid-session with live items", () => {
    const items = [toolItem(html("design:x", "<p>v1</p>"))];
    const first = renderHook(() => useDesignStore(items, "sess-1"));
    expect(first.result.current.designs[0].versions).toHaveLength(1);
    first.unmount();
    const second = renderHook(() => useDesignStore(items, "sess-1")); // same non-empty items
    expect(second.result.current.designs[0].versions).toHaveLength(1);
    second.unmount();
    const third = renderHook(() => useDesignStore(
      [...items, toolItem(html("design:x", "<p>v2</p>"))], "sess-1")); // one new render
    expect(third.result.current.designs[0].versions).toHaveLength(2);
  });

  it("re-seeds sent pins when sessionId changes while mounted", () => {
    const h = renderHook(({ sid }) => useDesignStore([], sid), { initialProps: { sid: "sess-a" } });
    act(() => h.result.current.recordSent("design:x", 1, [{ x_pct: 0.1, y_pct: 0.1, comment: "a" }]));
    h.rerender({ sid: "sess-b" });
    expect(h.result.current.sentPins("design:x", 1)).toEqual([]); // no bleed into sess-b
    const backA = JSON.parse(localStorage.getItem("agent.designs.sess-a")!);
    expect(backA.sent["design:x@1"]).toHaveLength(1); // sess-a data intact
    const rawB = localStorage.getItem("agent.designs.sess-b");
    if (rawB) expect(JSON.parse(rawB).sent["design:x@1"] ?? []).toEqual([]); // sess-b never got sess-a pins
  });
});
