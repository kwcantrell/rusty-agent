# Claude Design Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a third right-pane tab, **Design**, with a versioned annotate-on-canvas design surface (agent renders via the existing `render` tool with `design:<name>` ids) and a Tauri-only Config sub-section (system-prompt override + all runtime settings, applied at the next turn boundary).

**Architecture:** Approach A (artifact-native) from the spec `docs/superpowers/specs/2026-07-05-claude-design-tab-design.md`. Design versions are derived purely from the reducer's `Item[]` (every `tool_result` display whose id starts with `design:`), mirrored to localStorage per session, behind a `DesignStore` seam. Feedback is a fixed-schema chat message. Config rides the EXISTING `settings_get`/`settings_update` Tauri commands into `RuntimeState::apply` (validate → persist → next-turn loop swap) — no new commands.

**Tech Stack:** React 19 + Vite + Tailwind + vitest/@testing-library (web), Rust (agent/ workspace: `agent-runtime-config`, `agent-server`, `agent-tools`), Tauri 2 (src-tauri/ workspace).

## Deviations from the spec (all simplifications, agreed direction preserved)

1. **No new Tauri commands / no `apply_live`.** The spec proposed `get_runtime_config`/`set_runtime_config`/`apply_live`. Research found `RuntimeState::apply()` (agent-server/src/runtime.rs:130) already does validate → normalize → rebuild loop → persist → atomic swap, applied at the **next turn boundary** ("an in-flight run keeps the Arc it already cloned"), reachable via existing `settings_update`. That is the spec's "live where safe" semantics for ALL fields — strictly better than "next session" for structural ones. The only new backend capability is a `system_prompt_override` config field.
2. **"Dead-session degrade" for `apply_live` is moot** — there is no separate live path.
3. **Version stacks derive from reducer items** (like `artifactsFrom`) plus a localStorage mirror frozen at mount; the spec's "store intercepts the stream" is implemented as pure derivation behind the same `DesignStore` seam.

## Global Constraints

- Two Cargo workspaces: `agent/` and `src-tauri/`. Run `source ~/.cargo/env` first; `cargo … -p <crate>` must run in the right workspace directory.
- Conventional commits: `type(scope): summary`.
- Web commands run in `web/`: `npm test`, `npm run typecheck`.
- Feedback JSON schema is a FROZEN contract (B-migration): `{design_id, version, pins:[{x_pct,y_pct,comment}], note?}` — pinned by golden test, never change shape.
- Version stacks are bounded at `MAX_VERSIONS = 20` (oldest dropped).
- No wire-protocol changes (`ServerEvent`, `Outbound` unchanged). The only wire-visible change is the new `system_prompt_override` field riding the existing `RuntimeConfig` serialization.
- localStorage access always wrapped in try/catch (matches existing `storage.ts` style).
- The base-prompt ratchet test (`prompts.rs`) fails the build if "You are a local coding agent" is pasted into any other .rs file — never duplicate the prompt text.
- Final gate: `bash scripts/ci.sh` from repo root.

---

### Task 1: Third right-pane tab value ("design")

**Files:**
- Modify: `web/src/storage.ts:81-89`
- Modify: `web/src/components/RightPaneTabs.tsx`
- Test: `web/src/storage.rightTab.test.ts` (create)
- Test: `web/src/components/RightPaneTabs.test.tsx` (create)

**Interfaces:**
- Consumes: nothing new.
- Produces: `type RightTab = "workspace" | "context" | "design"` (storage.ts export, used by App in Task 6); `RightPaneTabs` renders a third tab labeled "Design".

- [ ] **Step 1: Write failing tests**

`web/src/storage.rightTab.test.ts`:

```ts
import { describe, it, expect, beforeEach } from "vitest";
import { loadRightTab, saveRightTab } from "./storage";

describe("right tab persistence", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to workspace", () => {
    expect(loadRightTab()).toBe("workspace");
  });

  it("round-trips design", () => {
    saveRightTab("design");
    expect(loadRightTab()).toBe("design");
  });

  it("round-trips context", () => {
    saveRightTab("context");
    expect(loadRightTab()).toBe("context");
  });

  it("falls back to workspace on a stale stored value", () => {
    localStorage.setItem("rightTab", "garbage");
    expect(loadRightTab()).toBe("workspace");
  });
});
```

`web/src/components/RightPaneTabs.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { RightPaneTabs } from "./RightPaneTabs";

describe("RightPaneTabs", () => {
  it("renders Workspace, Context, and Design tabs", () => {
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    expect(screen.getByRole("tab", { name: "Workspace" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Context" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Design" })).toBeInTheDocument();
  });

  it("selects Design on click", () => {
    const picked: string[] = [];
    render(<RightPaneTabs rightTab="workspace" setRightTab={(t) => picked.push(t)} />);
    fireEvent.click(screen.getByRole("tab", { name: "Design" }));
    expect(picked).toEqual(["design"]);
    expect(screen.getByRole("tab", { name: "Workspace" })).toHaveAttribute("aria-selected", "true");
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run storage.rightTab RightPaneTabs`
Expected: FAIL — `loadRightTab()` returns "workspace" for "design" (round-trip test), and no tab named "Design".

- [ ] **Step 3: Implement**

In `web/src/storage.ts`, replace the `RightTab` block:

```ts
const RIGHT_TAB = "rightTab";
export type RightTab = "workspace" | "context" | "design";
export function loadRightTab(): RightTab {
  try {
    const v = localStorage.getItem(RIGHT_TAB);
    return v === "context" || v === "design" ? v : "workspace";
  } catch { return "workspace"; }
}
export function saveRightTab(t: RightTab): void {
  try { localStorage.setItem(RIGHT_TAB, t); } catch { /* ignore */ }
}
```

In `web/src/components/RightPaneTabs.tsx`, replace the tab array and label:

```tsx
import type { RightTab } from "../storage";
import { saveRightTab } from "../storage";

const LABELS: Record<RightTab, string> = { workspace: "Workspace", context: "Context", design: "Design" };

export function RightPaneTabs(
  { rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void },
) {
  return (
    <div className="flex gap-1 px-2 pt-2" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
      {(["workspace", "context", "design"] as const).map((t) => (
        <button key={t} role="tab" aria-selected={rightTab === t}
          onClick={() => { setRightTab(t); saveRightTab(t); }}
          className="rounded-t-lg px-3 py-1.5 text-xs"
          style={{ color: rightTab === t ? "var(--text-strong)" : "var(--text-muted)",
            fontWeight: rightTab === t ? 600 : 400 }}>
          {LABELS[t]}
        </button>
      ))}
    </div>
  );
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run storage.rightTab RightPaneTabs` → PASS. Also `npm run typecheck` → clean.

- [ ] **Step 5: Commit**

```bash
git add web/src/storage.ts web/src/components/RightPaneTabs.tsx web/src/storage.rightTab.test.ts web/src/components/RightPaneTabs.test.tsx
git commit -m "feat(web): add Design as third right-pane tab"
```

---

### Task 2: Design store — version derivation, interception, localStorage mirror

**Files:**
- Create: `web/src/designStore.ts`
- Modify: `web/src/state.ts:319-331` (`artifactsFrom` skips design displays)
- Test: `web/src/designStore.test.ts` (create)
- Test: `web/src/state.ts` change covered in the same test file

**Interfaces:**
- Consumes: `Display` from `./wire`, `Item` from `./state` (type-only import — no runtime cycle; `state.ts` imports the `displayDesignId` function from here at runtime).
- Produces (used by Tasks 4–6):

```ts
export const MAX_VERSIONS = 20;
export interface Pin { x_pct: number; y_pct: number; comment: string }
export interface DesignVersion { display: Display; renderable: boolean }
export interface Design { id: string; title: string; versions: DesignVersion[] }
export function displayDesignId(d: Display): string | null;
export function designsFrom(items: Item[]): Design[];
export function mergeDesigns(stored: Design[], live: Design[]): Design[];
export interface DesignStoreApi {
  designs: Design[];
  sentPins: (designId: string, version: number) => Pin[];  // version is 1-based
  recordSent: (designId: string, version: number, pins: Pin[]) => void;
}
export function useDesignStore(items: Item[], sessionId: string): DesignStoreApi;
```

- [ ] **Step 1: Write failing tests**

`web/src/designStore.test.ts`:

```ts
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
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run designStore`
Expected: FAIL — module `./designStore` does not exist.

- [ ] **Step 3: Implement `web/src/designStore.ts`**

```ts
import { useEffect, useMemo, useState } from "react";
import type { Display } from "./wire";
import type { Item } from "./state";

export const MAX_VERSIONS = 20;

export interface Pin { x_pct: number; y_pct: number; comment: string }
export interface DesignVersion { display: Display; renderable: boolean }
export interface Design { id: string; title: string; versions: DesignVersion[] }

/** Variant keys ArtifactRenderer knows how to draw (mirror of its branches). */
const RENDERABLE = new Set(["Text", "Markdown", "Code", "Diff", "Terminal", "Table", "Image", "Html", "Mermaid"]);

/** The design id ("design:<name>") when this display targets the canvas, else null. */
export function displayDesignId(d: Display): string | null {
  const v = Object.values(d)[0] as { id?: string } | string;
  const id = v && typeof v === "object" ? v.id : undefined;
  return id !== undefined && id.startsWith("design:") ? id : null;
}

function displayTitle(d: Display): string | undefined {
  const v = Object.values(d)[0] as { title?: string } | string;
  return v && typeof v === "object" ? v.title : undefined;
}

function cap(d: Design): Design {
  return d.versions.length <= MAX_VERSIONS
    ? d
    : { ...d, versions: d.versions.slice(d.versions.length - MAX_VERSIONS) };
}

/** Pure derivation: every tool display with a design: id becomes a version, in order. */
export function designsFrom(items: Item[]): Design[] {
  const map = new Map<string, Design>();
  for (const it of items) {
    if (it.kind !== "tool" || !it.display) continue;
    const id = displayDesignId(it.display);
    if (!id) continue;
    const cur = map.get(id) ?? { id, title: id, versions: [] };
    cur.versions.push({
      display: it.display,
      renderable: RENDERABLE.has(Object.keys(it.display)[0]),
    });
    cur.title = displayTitle(it.display) ?? cur.title;
    map.set(id, cur);
  }
  return [...map.values()].map(cap);
}

/** Stored history (frozen at mount) followed by live-derived versions, capped. */
export function mergeDesigns(stored: Design[], live: Design[]): Design[] {
  const out = new Map<string, Design>(stored.map((d) => [d.id, d]));
  for (const l of live) {
    const s = out.get(l.id);
    out.set(l.id, s ? cap({ ...l, versions: [...s.versions, ...l.versions] }) : l);
  }
  return [...out.values()];
}

interface StoredBlob { designs: Design[]; sent: Record<string, Pin[]> }
const KEY = (sid: string) => `agent.designs.${sid}`;

function loadBlob(sid: string): StoredBlob {
  try {
    const raw = localStorage.getItem(KEY(sid));
    if (!raw) return { designs: [], sent: {} };
    const v = JSON.parse(raw) as Partial<StoredBlob>;
    return { designs: Array.isArray(v.designs) ? v.designs : [], sent: v.sent ?? {} };
  } catch { return { designs: [], sent: {} }; }
}

function saveBlob(sid: string, blob: StoredBlob): void {
  try { localStorage.setItem(KEY(sid), JSON.stringify(blob)); } catch { /* in-memory only */ }
}

export interface DesignStoreApi {
  designs: Design[];
  sentPins: (designId: string, version: number) => Pin[];
  recordSent: (designId: string, version: number, pins: Pin[]) => void;
}

/**
 * DesignStore v1: stored history is FROZEN at mount (so live derivation never
 * double-counts), merged with live items, mirrored back to localStorage.
 * The B migration swaps this hook's internals for a server-backed store.
 */
export function useDesignStore(items: Item[], sessionId: string): DesignStoreApi {
  // eslint-disable-next-line react-hooks/exhaustive-deps -- frozen-at-mount by design
  const stored = useMemo(() => loadBlob(sessionId), [sessionId]);
  const [sent, setSent] = useState<Record<string, Pin[]>>(stored.sent);
  const designs = useMemo(() => mergeDesigns(stored.designs, designsFrom(items)), [stored, items]);

  useEffect(() => { saveBlob(sessionId, { designs, sent }); }, [sessionId, designs, sent]);

  return {
    designs,
    sentPins: (id, version) => sent[`${id}@${version}`] ?? [],
    recordSent: (id, version, pins) =>
      setSent((s) => ({ ...s, [`${id}@${version}`]: [...(s[`${id}@${version}`] ?? []), ...pins] })),
  };
}
```

- [ ] **Step 4: Intercept in `web/src/state.ts`**

Add the import at the top of `state.ts`:

```ts
import { displayDesignId } from "./designStore";
```

Change `artifactsFrom`'s filter line from `if (it.kind === "tool" && it.display) {` to:

```ts
if (it.kind === "tool" && it.display && displayDesignId(it.display) === null) {
```

- [ ] **Step 5: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run designStore` → PASS. Then the full suite `npm test -- --run` — existing `state`/`App` tests must stay green (no existing test uses `design:` ids, so `artifactsFrom` behavior is unchanged for them).

- [ ] **Step 6: Commit**

```bash
git add web/src/designStore.ts web/src/designStore.test.ts web/src/state.ts
git commit -m "feat(web): design store — version derivation, artifact interception, session mirror"
```

---

### Task 3: Feedback message builder (frozen contract)

**Files:**
- Create: `web/src/designFeedback.ts`
- Test: `web/src/designFeedback.test.ts` (create)

**Interfaces:**
- Consumes: `Pin` from `./designStore`.
- Produces: `buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string): string` — used by `DesignPane` (Task 6). The fenced `design-feedback` JSON block is the B-migration contract.

- [ ] **Step 1: Write the failing golden test**

`web/src/designFeedback.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { buildFeedbackMessage } from "./designFeedback";

describe("buildFeedbackMessage", () => {
  it("serializes the exact frozen contract", () => {
    const msg = buildFeedbackMessage("design:landing", 3,
      [{ x_pct: 0.42, y_pct: 0.105, comment: "make the logo bigger" }],
      "overall: tighten vertical spacing");
    expect(msg).toBe(`Design feedback on design:landing (v3):

\`\`\`design-feedback
{
  "design_id": "design:landing",
  "version": 3,
  "pins": [
    {
      "x_pct": 0.42,
      "y_pct": 0.105,
      "comment": "make the logo bigger"
    }
  ],
  "note": "overall: tighten vertical spacing"
}
\`\`\``);
  });

  it("omits note when absent", () => {
    const msg = buildFeedbackMessage("design:x", 1, [{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]);
    expect(msg).not.toContain('"note"');
    expect(msg).toContain('"version": 1');
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run (in `web/`): `npm test -- --run designFeedback` → FAIL (module missing).

- [ ] **Step 3: Implement `web/src/designFeedback.ts`**

```ts
import type { Pin } from "./designStore";

/**
 * FROZEN CONTRACT (B-migration): this JSON shape becomes the DesignFeedback
 * tool-result payload when the first-class design channel lands. Field names
 * and structure must not change — the golden test pins the exact output.
 */
export function buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string): string {
  const payload: Record<string, unknown> = { design_id: designId, version, pins };
  if (note !== undefined && note.trim().length > 0) payload.note = note;
  return `Design feedback on ${designId} (v${version}):\n\n\`\`\`design-feedback\n${JSON.stringify(payload, null, 2)}\n\`\`\``;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run (in `web/`): `npm test -- --run designFeedback` → PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/designFeedback.ts web/src/designFeedback.test.ts
git commit -m "feat(web): design feedback message builder with frozen schema"
```

---

### Task 4: VersionBar + DesignCanvas

**Files:**
- Create: `web/src/components/design/VersionBar.tsx`
- Create: `web/src/components/design/DesignCanvas.tsx`
- Test: `web/src/components/design/DesignCanvas.test.tsx` (create)

**Interfaces:**
- Consumes: `Design`, `Pin` from `../../designStore`; `ArtifactRenderer` from `../inspector/ArtifactRenderer`; `AnnotationOverlay` from Task 5 (build order note: implement Task 5 FIRST if executing sequentially, or stub it — the plan orders the test run after Task 5's commit; alternatively execute Tasks 4+5 together. Recommended: implement both files, then run both test files).
- Produces:

```tsx
export function VersionBar(props: {
  current: number;            // 0-based index of the viewed version
  total: number;
  compare: boolean;
  renderableFlags: boolean[];
  onSelect: (i: number) => void;
  onLatest: () => void;
  onToggleCompare: () => void;
}): JSX.Element;

export function DesignCanvas(props: {
  design: Design;
  sentPins: (version: number) => Pin[];          // 1-based version
  onSendFeedback: (version: number, pins: Pin[]) => void;
  sendDisabled: boolean;
}): JSX.Element;
```

- [ ] **Step 1: Write failing tests**

`web/src/components/design/DesignCanvas.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DesignCanvas } from "./DesignCanvas";
import type { Design } from "../../designStore";

const design = (n: number): Design => ({
  id: "design:x", title: "X",
  versions: Array.from({ length: n }, (_, i) =>
    ({ display: { Html: { html: `<p>v${i + 1}</p>`, id: "design:x" } }, renderable: true })),
});

const noPins = () => [];

describe("DesignCanvas", () => {
  it("shows the latest version by default and follows new versions", () => {
    const { rerender } = render(<DesignCanvas design={design(2)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v2 / 2")).toBeInTheDocument();
    rerender(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v3 / 3")).toBeInTheDocument();
  });

  it("steps back and shows a new-version badge instead of yanking the view", () => {
    const { rerender } = render(<DesignCanvas design={design(2)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    fireEvent.click(screen.getByRole("button", { name: "previous version" }));
    expect(screen.getByText("v1 / 2")).toBeInTheDocument();
    rerender(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText("v1 / 3")).toBeInTheDocument(); // view not yanked
    const badge = screen.getByRole("button", { name: /v3 available/ });
    fireEvent.click(badge);
    expect(screen.getByText("v3 / 3")).toBeInTheDocument();
  });

  it("compare mode renders the previous and current versions side by side", () => {
    render(<DesignCanvas design={design(3)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Compare" }));
    expect(screen.getByTestId("compare-left")).toBeInTheDocument();
    expect(screen.getByTestId("compare-right")).toBeInTheDocument();
  });

  it("compare is disabled with a single version", () => {
    render(<DesignCanvas design={design(1)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByRole("button", { name: "Compare" })).toBeDisabled();
  });

  it("marks an unsupported version", () => {
    const d = design(1);
    d.versions[0] = { display: { Frob: { x: 1 } } as never, renderable: false };
    render(<DesignCanvas design={d} sentPins={noPins} onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByText(/unsupported/)).toBeInTheDocument();
  });

  it("passes 1-based version numbers to feedback", () => {
    const sent = vi.fn();
    render(<DesignCanvas design={design(2)} sentPins={noPins} onSendFeedback={sent} sendDisabled={false} />);
    // AnnotationOverlay is exercised in its own test; here we only pin the wiring
    // by checking the sentPins accessor version (asserted via the overlay's sent markers).
    expect(sent).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run DesignCanvas` → FAIL (modules missing).

- [ ] **Step 3: Implement `web/src/components/design/VersionBar.tsx`**

```tsx
export function VersionBar({ current, total, compare, renderableFlags, onSelect, onLatest, onToggleCompare }: {
  current: number; total: number; compare: boolean; renderableFlags: boolean[];
  onSelect: (i: number) => void; onLatest: () => void; onToggleCompare: () => void;
}) {
  const btn = "rounded px-2 py-0.5 text-xs disabled:opacity-40 disabled:cursor-not-allowed";
  return (
    <div className="flex items-center gap-2 px-3 py-2" style={{ borderBottom: "1px solid var(--border)" }}>
      <button aria-label="previous version" className={btn} disabled={current === 0}
        onClick={() => onSelect(current - 1)} style={{ color: "var(--text-muted)" }}>←</button>
      <span className="text-xs" style={{ color: "var(--text-strong)" }}>
        v{current + 1} / {total}{renderableFlags[current] ? "" : " (unsupported)"}
      </span>
      <button aria-label="next version" className={btn} disabled={current >= total - 1}
        onClick={() => onSelect(current + 1)} style={{ color: "var(--text-muted)" }}>→</button>
      <button className={btn} disabled={current >= total - 1} onClick={onLatest}
        style={{ color: "var(--text-muted)" }}>latest</button>
      <div className="flex-1" />
      <button aria-pressed={compare} className={btn} disabled={total < 2} onClick={onToggleCompare}
        style={{ background: compare ? "var(--accent)" : "transparent",
          color: compare ? "var(--accent-fg)" : "var(--text-muted)",
          border: "1px solid var(--border)" }}>Compare</button>
    </div>
  );
}
```

- [ ] **Step 4: Implement `web/src/components/design/DesignCanvas.tsx`**

```tsx
import { useState } from "react";
import type { Design, Pin } from "../../designStore";
import { ArtifactRenderer } from "../inspector/ArtifactRenderer";
import { VersionBar } from "./VersionBar";
import { AnnotationOverlay } from "./AnnotationOverlay";

export function DesignCanvas({ design, sentPins, onSendFeedback, sendDisabled }: {
  design: Design;
  sentPins: (version: number) => Pin[];
  onSendFeedback: (version: number, pins: Pin[]) => void;
  sendDisabled: boolean;
}) {
  const [viewed, setViewed] = useState<number | null>(null); // null = follow latest
  const [compare, setCompare] = useState(false);
  const total = design.versions.length;
  const cur = Math.min(viewed ?? total - 1, total - 1);
  const behind = viewed !== null && cur < total - 1;
  return (
    <div className="flex h-full min-h-0 flex-col">
      <VersionBar current={cur} total={total} compare={compare}
        renderableFlags={design.versions.map((v) => v.renderable)}
        onSelect={setViewed} onLatest={() => setViewed(null)}
        onToggleCompare={() => setCompare((c) => !c)} />
      {behind && (
        <button onClick={() => setViewed(null)}
          className="mx-3 mt-2 rounded px-2 py-1 text-xs"
          style={{ background: "var(--surface-raised)", color: "var(--text-strong)",
            border: "1px solid var(--border)" }}>
          v{total} available — jump to latest
        </button>
      )}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {compare && cur > 0 ? (
          <div className="flex h-full gap-2">
            <div className="min-w-0 flex-1" data-testid="compare-left">
              <ArtifactRenderer display={design.versions[cur - 1].display} />
            </div>
            <div className="min-w-0 flex-1" data-testid="compare-right">
              <ArtifactRenderer display={design.versions[cur].display} />
            </div>
          </div>
        ) : (
          <AnnotationOverlay sent={sentPins(cur + 1)} disabled={sendDisabled}
            onSend={(pins) => onSendFeedback(cur + 1, pins)}>
            <ArtifactRenderer display={design.versions[cur].display} />
          </AnnotationOverlay>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Run tests (after Task 5's overlay exists)**

Run (in `web/`): `npm test -- --run DesignCanvas` → PASS (execute together with Task 5 if the overlay import blocks compilation).

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/VersionBar.tsx web/src/components/design/DesignCanvas.tsx web/src/components/design/DesignCanvas.test.tsx
git commit -m "feat(web): design canvas with version stepping, compare, new-version badge"
```

---

### Task 5: AnnotationOverlay

**Files:**
- Create: `web/src/components/design/AnnotationOverlay.tsx`
- Test: `web/src/components/design/AnnotationOverlay.test.tsx` (create)

**Interfaces:**
- Consumes: `Pin` from `../../designStore`.
- Produces:

```tsx
export function AnnotationOverlay(props: {
  children: React.ReactNode;   // the rendered artifact
  sent: Pin[];                 // pins already sent for this version (read-only markers)
  disabled: boolean;
  onSend: (pins: Pin[]) => void;  // drafts with non-empty comments
}): JSX.Element;
```

- [ ] **Step 1: Write failing tests**

`web/src/components/design/AnnotationOverlay.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AnnotationOverlay } from "./AnnotationOverlay";

// jsdom has no layout: give the pin layer a fake box so pct math works.
function mockBox(el: HTMLElement) {
  vi.spyOn(el, "getBoundingClientRect").mockReturnValue({
    left: 0, top: 0, width: 200, height: 100, right: 200, bottom: 100, x: 0, y: 0, toJSON: () => ({}),
  } as DOMRect);
}

describe("AnnotationOverlay", () => {
  it("click adds a draft pin at pct coords; comment enables send", () => {
    const onSend = vi.fn();
    render(<AnnotationOverlay sent={[]} disabled={false} onSend={onSend}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 100, clientY: 25 });
    expect(screen.getAllByTestId("pin-draft")).toHaveLength(1);

    const send = screen.getByRole("button", { name: /Send feedback/ });
    expect(send).toBeDisabled(); // empty comment
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "move this" } });
    expect(send).toBeEnabled();
    fireEvent.click(send);
    expect(onSend).toHaveBeenCalledWith([{ x_pct: 0.5, y_pct: 0.25, comment: "move this" }]);
    expect(screen.queryAllByTestId("pin-draft")).toHaveLength(0); // drafts cleared
  });

  it("deletes a draft pin", () => {
    render(<AnnotationOverlay sent={[]} disabled={false} onSend={() => {}}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 10, clientY: 10 });
    fireEvent.click(screen.getByRole("button", { name: "delete pin 1" }));
    expect(screen.queryAllByTestId("pin-draft")).toHaveLength(0);
  });

  it("renders sent pins as read-only markers", () => {
    render(<AnnotationOverlay sent={[{ x_pct: 0.1, y_pct: 0.2, comment: "done" }]} disabled={false}
      onSend={() => {}}><p>art</p></AnnotationOverlay>);
    expect(screen.getAllByTestId("pin-sent")).toHaveLength(1);
  });

  it("send stays disabled when the composer is disabled", () => {
    render(<AnnotationOverlay sent={[]} disabled={true} onSend={() => {}}><p>art</p></AnnotationOverlay>);
    const layer = screen.getByTestId("pin-layer");
    mockBox(layer.parentElement as HTMLElement);
    fireEvent.click(layer, { clientX: 10, clientY: 10 });
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "x" } });
    expect(screen.getByRole("button", { name: /Send feedback/ })).toBeDisabled();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run AnnotationOverlay` → FAIL (module missing).

- [ ] **Step 3: Implement `web/src/components/design/AnnotationOverlay.tsx`**

```tsx
import { useRef, useState, type ReactNode, type MouseEvent } from "react";
import type { Pin } from "../../designStore";

/**
 * Click-to-pin layer over a rendered artifact. Drafts are local until "Send
 * feedback"; pct coordinates are relative to the artifact box so they survive
 * pane resizes. The layer sits ABOVE iframe-hosted HTML, so no cross-frame
 * event wrangling (and mockups are intentionally non-interactive).
 */
export function AnnotationOverlay({ children, sent, disabled, onSend }: {
  children: ReactNode; sent: Pin[]; disabled: boolean; onSend: (pins: Pin[]) => void;
}) {
  const [drafts, setDrafts] = useState<Pin[]>([]);
  const box = useRef<HTMLDivElement>(null);

  const addPin = (e: MouseEvent) => {
    const r = box.current?.getBoundingClientRect();
    if (!r || r.width === 0 || r.height === 0) return;
    const x = Math.round(((e.clientX - r.left) / r.width) * 1000) / 1000;
    const y = Math.round(((e.clientY - r.top) / r.height) * 1000) / 1000;
    setDrafts((d) => [...d, { x_pct: x, y_pct: y, comment: "" }]);
  };
  const setComment = (i: number, comment: string) =>
    setDrafts((d) => d.map((p, j) => (j === i ? { ...p, comment } : p)));
  const remove = (i: number) => setDrafts((d) => d.filter((_, j) => j !== i));
  const ready = drafts.filter((p) => p.comment.trim().length > 0);
  const send = () => { onSend(ready); setDrafts([]); };

  return (
    <div className="flex h-full flex-col">
      <div ref={box} className="relative min-h-0 flex-1">
        {children}
        <div data-testid="pin-layer" className="absolute inset-0 cursor-crosshair" onClick={addPin}>
          {sent.map((p, i) => <Marker key={`s${i}`} pin={p} kind="sent" n={i + 1} />)}
          {drafts.map((p, i) => <Marker key={`d${i}`} pin={p} kind="draft" n={sent.length + i + 1} />)}
        </div>
      </div>
      <div className="space-y-1 p-2" style={{ borderTop: "1px solid var(--border)" }}>
        {drafts.map((p, i) => (
          <div key={i} className="flex items-center gap-2">
            <span className="text-xs" style={{ color: "var(--text-muted)" }}>#{sent.length + i + 1}</span>
            <input aria-label={`pin ${i + 1} comment`} value={p.comment}
              placeholder="what should change here?"
              className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
              style={{ background: "var(--surface-base)", color: "var(--text-strong)",
                border: "1px solid var(--border)" }}
              onChange={(e) => setComment(i, e.target.value)} />
            <button aria-label={`delete pin ${i + 1}`} onClick={() => remove(i)}
              className="text-xs" style={{ color: "var(--text-muted)" }}>✕</button>
          </div>
        ))}
        <button onClick={send} disabled={disabled || ready.length === 0}
          className="w-full rounded px-3 py-1.5 text-xs font-medium hover:opacity-90 disabled:opacity-40"
          style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>
          Send feedback{ready.length > 0 ? ` (${ready.length})` : ""}
        </button>
      </div>
    </div>
  );
}

function Marker({ pin, kind, n }: { pin: Pin; kind: "sent" | "draft"; n: number }) {
  return (
    <span data-testid={`pin-${kind}`} onClick={(e) => e.stopPropagation()} title={pin.comment}
      className="absolute flex h-5 w-5 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full text-[10px] font-bold"
      style={{ left: `${pin.x_pct * 100}%`, top: `${pin.y_pct * 100}%`,
        background: kind === "draft" ? "var(--accent)" : "var(--surface-raised)",
        color: kind === "draft" ? "var(--accent-fg)" : "var(--text-muted)",
        border: "1px solid var(--border)" }}>{n}</span>
  );
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `web/`): `npm test -- --run AnnotationOverlay DesignCanvas` → both PASS (Task 4's tests unblock here).

- [ ] **Step 5: Commit**

```bash
git add web/src/components/design/AnnotationOverlay.tsx web/src/components/design/AnnotationOverlay.test.tsx
git commit -m "feat(web): annotate-on-canvas overlay with draft pins and structured send"
```

---

### Task 6: DesignPane + App wiring

**Files:**
- Create: `web/src/components/design/DesignPane.tsx`
- Modify: `web/src/App.tsx` (third-tab rendering; extract the duplicated right-pane JSX)
- Test: `web/src/components/design/DesignPane.test.tsx` (create)

**Interfaces:**
- Consumes: `useDesignStore`, `buildFeedbackMessage`, `DesignCanvas`, `isTauri` from `../../transport`, `ConfigPanel` from Task 10 (until Task 10 lands, `DesignPane` renders a "Config" placeholder — this task creates the sub-nav and gating; Task 10 fills the panel).
- Produces:

```tsx
export interface DesignPaneProps {
  items: Item[];
  sessionId: string;
  onSend: (text: string) => void;
  sendDisabled: boolean;
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[];
    discoveredSkills: DiscoveredSkill[] } | null;
  settingsError: string | null;
  onSaveSettings: (s: RuntimeSettings) => void;
  onLoadSettings: () => void;
}
export function DesignPane(props: DesignPaneProps): JSX.Element;
```

- [ ] **Step 1: Write failing tests**

`web/src/components/design/DesignPane.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { Item } from "../../state";

const tauriMock = vi.hoisted(() => ({ value: true }));
vi.mock("../../transport", () => ({ isTauri: () => tauriMock.value }));

import { DesignPane } from "./DesignPane";

const designItem = (html: string): Item =>
  ({ kind: "tool", name: "render", args: {}, status: "done",
     display: { Html: { html, id: "design:landing", title: "Landing" } } });

const base = {
  sessionId: "s1", onSend: () => {}, sendDisabled: false,
  settings: null, settingsMeta: null, settingsError: null,
  onSaveSettings: () => {}, onLoadSettings: () => {},
};

describe("DesignPane", () => {
  beforeEach(() => { localStorage.clear(); tauriMock.value = true; });

  it("shows an empty state with no designs", () => {
    render(<DesignPane {...base} items={[]} />);
    expect(screen.getByText(/No designs yet/)).toBeInTheDocument();
  });

  it("renders the latest design version in the canvas", () => {
    render(<DesignPane {...base} items={[designItem("<p>v1</p>"), designItem("<p>v2</p>")]} />);
    expect(screen.getByText("v2 / 2")).toBeInTheDocument();
  });

  it("sends structured feedback and records sent pins", () => {
    const sent: string[] = [];
    render(<DesignPane {...base} items={[designItem("<p>v1</p>")]} onSend={(t) => sent.push(t)} />);
    const layer = screen.getByTestId("pin-layer");
    vi.spyOn(layer.parentElement as HTMLElement, "getBoundingClientRect").mockReturnValue({
      left: 0, top: 0, width: 100, height: 100, right: 100, bottom: 100, x: 0, y: 0, toJSON: () => ({}),
    } as DOMRect);
    fireEvent.click(layer, { clientX: 50, clientY: 50 });
    fireEvent.change(screen.getByLabelText("pin 1 comment"), { target: { value: "bigger" } });
    fireEvent.click(screen.getByRole("button", { name: /Send feedback/ }));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toContain("```design-feedback");
    expect(sent[0]).toContain('"design_id": "design:landing"');
    expect(screen.getAllByTestId("pin-sent")).toHaveLength(1); // retained as sent
  });

  it("shows the Config sub-tab under Tauri and loads settings on open", () => {
    const onLoad = vi.fn();
    render(<DesignPane {...base} items={[]} onLoadSettings={onLoad} />);
    fireEvent.click(screen.getByRole("tab", { name: "Config" }));
    expect(onLoad).toHaveBeenCalled();
  });

  it("hides the Config sub-tab entirely outside Tauri", () => {
    tauriMock.value = false;
    render(<DesignPane {...base} items={[]} />);
    expect(screen.queryByRole("tab", { name: "Config" })).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run DesignPane` → FAIL (module missing).

- [ ] **Step 3: Implement `web/src/components/design/DesignPane.tsx`**

```tsx
import { useState } from "react";
import type { Item } from "../../state";
import type { RuntimeSettings, DiscoveredSkill } from "../../wire";
import { isTauri } from "../../transport";
import { useDesignStore } from "../../designStore";
import { buildFeedbackMessage } from "../../designFeedback";
import { DesignCanvas } from "./DesignCanvas";
import { ConfigPanel } from "./ConfigPanel";

export interface DesignPaneProps {
  items: Item[];
  sessionId: string;
  onSend: (text: string) => void;
  sendDisabled: boolean;
  settings: RuntimeSettings | null;
  settingsMeta: { workspace: string; apiKeySet: boolean; hardFloor: string[];
    discoveredSkills: DiscoveredSkill[] } | null;
  settingsError: string | null;
  onSaveSettings: (s: RuntimeSettings) => void;
  onLoadSettings: () => void;
}

export function DesignPane({ items, sessionId, onSend, sendDisabled,
  settings, settingsMeta, settingsError, onSaveSettings, onLoadSettings }: DesignPaneProps) {
  const [section, setSection] = useState<"canvas" | "config">("canvas");
  const store = useDesignStore(items, sessionId);
  const [activeId, setActiveId] = useState<string | null>(null);
  const tauri = isTauri();
  const active = store.designs.find((d) => d.id === activeId) ?? store.designs[store.designs.length - 1];
  const sub = (on: boolean) => ({
    color: on ? "var(--text-strong)" : "var(--text-muted)", fontWeight: on ? 600 : 400,
  });

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      <div className="flex gap-1 px-2 pt-1" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
        <button role="tab" aria-selected={section === "canvas"} onClick={() => setSection("canvas")}
          className="rounded-t-lg px-3 py-1 text-xs" style={sub(section === "canvas")}>Canvas</button>
        {tauri && (
          <button role="tab" aria-selected={section === "config"}
            onClick={() => { setSection("config"); onLoadSettings(); }}
            className="rounded-t-lg px-3 py-1 text-xs" style={sub(section === "config")}>Config</button>
        )}
      </div>
      {section === "config" && tauri ? (
        <ConfigPanel settings={settings} meta={settingsMeta} error={settingsError}
          disabled={sendDisabled} onSave={onSaveSettings} />
      ) : !active ? (
        <div className="flex flex-1 items-center justify-center p-6 text-center text-sm"
          style={{ color: "var(--text-muted)" }}>
          <p>No designs yet. Ask the agent to render one with id &quot;design:&lt;name&gt;&quot;.</p>
        </div>
      ) : (
        <>
          {store.designs.length > 1 && (
            <div className="flex gap-1 overflow-x-auto px-2 pt-1" role="tablist">
              {store.designs.map((d) => (
                <button key={d.id} role="tab" aria-selected={d.id === active.id}
                  onClick={() => setActiveId(d.id)}
                  className="whitespace-nowrap rounded-t px-2 py-1 text-xs" style={sub(d.id === active.id)}>
                  {d.title}
                </button>
              ))}
            </div>
          )}
          <DesignCanvas key={active.id} design={active}
            sentPins={(v) => store.sentPins(active.id, v)}
            onSendFeedback={(v, pins) => {
              onSend(buildFeedbackMessage(active.id, v, pins));
              store.recordSent(active.id, v, pins);
            }}
            sendDisabled={sendDisabled} />
        </>
      )}
    </div>
  );
}
```

Until Task 10 lands, create a minimal placeholder `web/src/components/design/ConfigPanel.tsx` so this compiles (Task 10 replaces its body):

```tsx
import type { RuntimeSettings, DiscoveredSkill } from "../../wire";

export interface ConfigPanelProps {
  settings: RuntimeSettings | null;
  meta: { workspace: string; apiKeySet: boolean; hardFloor: string[];
    discoveredSkills: DiscoveredSkill[] } | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
}

export function ConfigPanel({ settings }: ConfigPanelProps) {
  return (
    <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>
      {settings ? "Config editor coming in Task 10." : "Loading settings…"}
    </p>
  );
}
```

- [ ] **Step 4: Wire into `web/src/App.tsx`**

Add imports:

```tsx
import { DesignPane } from "./components/design/DesignPane";
```

Inside `App()` after `const model = state.settings?.model;`, build the right pane ONCE (replacing the duplicated blocks in the wide and narrow branches):

```tsx
const rightPane = (
  <div className="flex h-full flex-col">
    <RightPaneTabs rightTab={rightTab} setRightTab={setRightTab} />
    <div className="min-h-0 flex-1">
      {rightTab === "workspace"
        ? <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
        : rightTab === "context"
          ? <ContextExplorer realTotal={state.serverUsage?.promptTokens ?? null} refreshKey={state.turnIndex}
              skills={state.settingsMeta?.discoveredSkills ?? []} lastQuery={lastQuery} />
          : <DesignPane items={state.items} sessionId={sessionId} onSend={send} sendDisabled={!connected}
              settings={state.settings} settingsMeta={state.settingsMeta} settingsError={state.settingsError}
              onSaveSettings={saveSettings}
              onLoadSettings={() => sock.current?.send({ kind: "settings_get" })} />}
    </div>
  </div>
);
```

Then the wide branch becomes:

```tsx
{!narrow && (
  <div className="min-w-0 flex-1">
    {rightPane}
  </div>
)}
```

and the narrow overlay's inner `div.h-[calc(100%-2.5rem)]` content becomes `{rightPane}` (keeping the close-button header above it).

- [ ] **Step 5: Run tests**

Run (in `web/`): `npm test -- --run` (full suite) and `npm run typecheck`.
Expected: all PASS — App tests exercise the default workspace tab and must be untouched by the refactor.

- [ ] **Step 6: Commit**

```bash
git add web/src/components/design/DesignPane.tsx web/src/components/design/DesignPane.test.tsx web/src/components/design/ConfigPanel.tsx web/src/App.tsx
git commit -m "feat(web): Design pane with canvas sub-section wired into the right pane"
```

---

### Task 7: `render` tool description documents the design: convention

**Files:**
- Modify: `agent/crates/agent-tools/src/render.rs:24-26`
- Test: same file, tests module

**Interfaces:**
- Consumes/Produces: behavior unchanged; description text only (the agent learns the canvas workflow from the tool schema).

- [ ] **Step 1: Write the failing test** (in `render.rs`'s existing `#[cfg(test)] mod tests`)

```rust
#[test]
fn description_documents_the_design_canvas_convention() {
    let t = RenderArtifact;
    assert!(t.description().contains("design:"), "agents must learn the design canvas from the schema");
    assert!(t.schema().parameters["properties"]["id"]["description"]
        .as_str().unwrap().contains("design:"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run (in `agent/`): `source ~/.cargo/env && cargo test -p agent-tools description_documents` → FAIL.

- [ ] **Step 3: Implement**

Update `description()`:

```rust
fn description(&self) -> &str {
    "Render an artifact (markdown, code, html, mermaid diagram, table, or image) into the user's \
     Inspector panel. For iterative visual design, use an id starting with `design:` (e.g. \
     `design:landing-page`): each re-render of that id adds a new version to the user's Design \
     canvas, where they can step through versions, compare them, and pin feedback that comes \
     back to you as a `design-feedback` message."
}
```

Update the `id` property description in `schema()`:

```rust
"id": {"type": "string", "description": "stable id; re-rendering the same id replaces the artifact. Ids starting with `design:` version on the Design canvas instead of replacing."},
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `agent/`): `cargo test -p agent-tools` → PASS (all, not just the new one).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/render.rs
git commit -m "docs(tools): render tool teaches the design: canvas convention"
```

---

### Task 8: `system_prompt_override` config field (Rust)

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs` (field, Partial mirror, merge arm, normalized(), default-construction sites)
- Modify: `agent/crates/agent-runtime-config/src/assemble.rs:165-175` (use the override as the compose base)
- Test: both files' tests modules

**Interfaces:**
- Consumes: nothing new.
- Produces: `RuntimeConfig.system_prompt_override: Option<String>` — flows automatically through `SettingsState`/`settings_update` (RuntimeConfig serializes onto the wire) and through `RuntimeState::apply` → `assemble_loop` (system prompt recomposed on every apply; applied at next turn boundary via `current_system_prompt()`).

- [ ] **Step 1: Write failing tests**

In `runtime_config.rs` tests module:

```rust
#[test]
fn system_prompt_override_round_trips_and_merges() {
    let mut cfg = test_config(); // use the existing test-config helper in this module
    cfg.system_prompt_override = Some("You are a design assistant.".into());
    let json = serde_json::to_string(&cfg).unwrap();
    let back: RuntimeConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.system_prompt_override.as_deref(), Some("You are a design assistant."));

    // old on-disk config without the field → None
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("system_prompt_override");
    let old: RuntimeConfig = serde_json::from_value(v).unwrap();
    assert!(old.system_prompt_override.is_none());
}

#[test]
fn normalized_maps_blank_override_to_none() {
    let mut cfg = test_config();
    cfg.system_prompt_override = Some("   \n".into());
    assert!(cfg.normalized().system_prompt_override.is_none());
}
```

In `assemble.rs` tests module (pattern-match the existing `assemble_loop` test around line 382 that sets `base_system_prompt: "BASE"`):

```rust
#[test]
fn system_prompt_override_replaces_the_base() {
    let mut cfg = test_cfg(); // this module's existing config helper
    cfg.system_prompt_override = Some("OVERRIDE PROMPT".into());
    let parts = test_parts(); // existing helper with base_system_prompt: "BASE"
    let built = assemble_loop(&cfg, parts);
    assert!(built.system_prompt.starts_with("OVERRIDE PROMPT"));
    assert!(!built.system_prompt.contains("BASE"));
}
```

(Adapt helper names to what the module actually uses — read the neighboring tests first; both files have established helpers.)

- [ ] **Step 2: Run tests to verify they fail**

Run (in `agent/`): `cargo test -p agent-runtime-config` → compile FAIL (unknown field), which is the expected failure mode.

- [ ] **Step 3: Implement**

In `runtime_config.rs`, add to `RuntimeConfig` (after `trace_max_mb`):

```rust
/// When set, replaces the built-in base system prompt. Active skills and
/// preset text still append on top. Blank strings normalize to None.
#[serde(default)]
pub system_prompt_override: Option<String>,
```

Add to `PartialRuntimeConfig`:

```rust
system_prompt_override: Option<String>,
```

Add a merge arm at the end of `merge()` (before `self`):

```rust
if let Some(v) = p.system_prompt_override {
    self.system_prompt_override = Some(v);
}
```

Extend `normalized()`:

```rust
pub fn normalized(mut self) -> Self {
    if self.backend == "claude-cli" {
        self.protocol = "prompted".into();
    }
    if self.system_prompt_override.as_deref().is_some_and(|s| s.trim().is_empty()) {
        self.system_prompt_override = None;
    }
    self
}
```

Then `cargo build -p agent-runtime-config` — the compiler lists every literal `RuntimeConfig { … }` construction site (there is at least the defaults constructor near line 300, plus CLI flag-derived builders and test fixtures across crates). Add `system_prompt_override: None,` to each. **Note:** the repo has an all-fields structural guard test for the Partial mirror (memory: "memory partial-merge gap"); it fails if the Partial field or merge arm is missing — let it verify you.

In `assemble.rs`, change the compose site (~line 165):

```rust
let base: &str = cfg
    .system_prompt_override
    .as_deref()
    .unwrap_or(&parts.base_system_prompt);
let system_prompt = match compose_system_prompt(
    base,
    // …existing args unchanged…
) {
    // …
    Err(e) => {
        tracing::error!(error = %e, "compose_system_prompt failed unexpectedly; using base prompt");
        base.to_string()
    }
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `agent/`): `cargo test -p agent-runtime-config && cargo build` (whole workspace — catches construction sites in other crates). Then `cd ../src-tauri && cargo build` for the second workspace. All green.

- [ ] **Step 5: Commit**

```bash
git add -A agent/ src-tauri/
git commit -m "feat(config): system_prompt_override field threaded through loop assembly"
```

---

### Task 9: External-edit guard in `RuntimeState::apply`

**Files:**
- Modify: `agent/crates/agent-server/src/runtime.rs` (new field + guard in `apply`)
- Test: same file, tests module

**Interfaces:**
- Consumes: existing `config_path` field.
- Produces: `apply()` returns `Err("config file changed externally — restart the daemon or re-save from the CLI")` when the on-disk config no longer matches what this daemon last read/wrote. Surfaces in the UI through the existing `settings_error` path — no wire change.

- [ ] **Step 1: Write failing tests** (in `runtime.rs`'s tests module, using its existing `RuntimeState` fixture — read `apply_swaps_the_loop_and_persists` (line ~283) and reuse its setup)

```rust
#[test]
fn apply_refuses_when_config_file_changed_externally() {
    let (rs, dir) = test_runtime_state(); // existing fixture helper; adapt to its real name
    let next = rs.settings_state().settings;
    rs.apply(next.clone()).unwrap(); // first apply persists

    // simulate a CLI edit behind the daemon's back
    std::fs::write(dir.path().join("config.json"), "{\"model\":\"other\"}").unwrap();

    let err = rs.apply(next).unwrap_err();
    assert!(err.contains("changed externally"), "got: {err}");
}

#[test]
fn apply_twice_without_external_edits_is_fine() {
    let (rs, _dir) = test_runtime_state();
    let next = rs.settings_state().settings;
    rs.apply(next.clone()).unwrap();
    rs.apply(next).unwrap();
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run (in `agent/`): `cargo test -p agent-server apply_refuses` → FAIL (no guard yet; first test's second apply succeeds).

- [ ] **Step 3: Implement**

Add a field to `RuntimeState`:

```rust
/// On-disk config content as of our last read/write. `apply` refuses to
/// clobber a file some other process (CLI, editor) changed since.
persisted_file: Mutex<Option<String>>,
```

In `new()`, initialize (before constructing `Self`):

```rust
let persisted_file = Mutex::new(std::fs::read_to_string(&config_path).ok());
```

and add `persisted_file,` to the `Self { … }` literal.

In `apply()`, insert after `cfg.validate()?;` (before the expensive `build_loop`):

```rust
{
    let seen = self.persisted_file.lock().unwrap();
    let on_disk = std::fs::read_to_string(&self.config_path).ok();
    if on_disk != *seen {
        return Err(
            "config file changed externally — restart the daemon or re-save from the CLI".into(),
        );
    }
}
```

After the successful `cfg.save(&self.config_path)…?;`, record the new content:

```rust
*self.persisted_file.lock().unwrap() = std::fs::read_to_string(&self.config_path).ok();
```

- [ ] **Step 4: Run tests to verify they pass**

Run (in `agent/`): `cargo test -p agent-server` → PASS (including the pre-existing apply tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-server/src/runtime.rs
git commit -m "feat(server): refuse settings apply when config file changed externally"
```

---

### Task 10: SettingsForm extraction + ConfigPanel (system prompt + full settings in the Design tab)

**Files:**
- Create: `web/src/components/SettingsForm.tsx` (extracted from SettingsPanel + new System prompt section)
- Modify: `web/src/components/SettingsPanel.tsx` (becomes overlay shell around SettingsForm)
- Modify: `web/src/components/design/ConfigPanel.tsx` (replace Task 6 placeholder)
- Modify: `web/src/wire.ts:14-38` (`RuntimeSettings` + `system_prompt_override`)
- Test: `web/src/components/design/ConfigPanel.test.tsx` (create)
- Test: existing `web/src/components/SettingsPanel.test.tsx` must stay green (the extraction is behavior-preserving)

**Interfaces:**
- Consumes: `RuntimeSettings` (now with `system_prompt_override: string | null`), `SettingsState` meta shape.
- Produces:

```tsx
// SettingsForm.tsx
export interface SettingsMeta { workspace: string; apiKeySet: boolean; hardFloor: string[];
  discoveredSkills: { name: string; description: string }[] }
export function SettingsForm(props: {
  settings: RuntimeSettings; meta: SettingsMeta | null; error: string | null;
  disabled: boolean; onSave: (s: RuntimeSettings) => void;
}): JSX.Element;

// ConfigPanel.tsx — same ConfigPanelProps as the Task 6 placeholder.
```

- [ ] **Step 1: Add the wire field**

In `web/src/wire.ts`, add to `RuntimeSettings` (after `trace_max_mb`):

```ts
system_prompt_override: string | null;
```

Run `npm run typecheck` — fix any fixture objects that construct a full `RuntimeSettings` in tests (add `system_prompt_override: null`).

- [ ] **Step 2: Write failing tests**

`web/src/components/design/ConfigPanel.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ConfigPanel } from "./ConfigPanel";
import type { RuntimeSettings } from "../../wire";

const settings: RuntimeSettings = {
  backend: "openai", base_url: "http://localhost:8080/v1", model: "m", protocol: "native",
  command_allowlist: [], command_denylist: [], temperature: 0.7, max_tokens: 1024,
  max_turns: 10, context_limit: 32768, top_p: null, top_k: null, min_p: null,
  presence_penalty: null, repeat_penalty: null, enable_thinking: true, preserve_thinking: false,
  memory: false, skills_dirs: [], active_skills: [], trace: true, trace_dir: null,
  trace_max_mb: 100, system_prompt_override: null,
};

describe("ConfigPanel", () => {
  it("shows loading before settings arrive", () => {
    render(<ConfigPanel settings={null} meta={null} error={null} disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/Loading settings/)).toBeInTheDocument();
  });

  it("edits the system prompt override and saves the full settings object", () => {
    const saved: RuntimeSettings[] = [];
    render(<ConfigPanel settings={settings} meta={null} error={null} disabled={false}
      onSave={(s) => saved.push(s)} />);
    fireEvent.change(screen.getByLabelText(/Override/), { target: { value: "You are a designer." } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(saved[0].system_prompt_override).toBe("You are a designer.");
    expect(saved[0].model).toBe("m"); // full object round-trips — nothing clobbered
  });

  it("empty override saves as null", () => {
    const saved: RuntimeSettings[] = [];
    render(<ConfigPanel settings={{ ...settings, system_prompt_override: "old" }} meta={null}
      error={null} disabled={false} onSave={(s) => saved.push(s)} />);
    fireEvent.change(screen.getByLabelText(/Override/), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(saved[0].system_prompt_override).toBeNull();
  });

  it("surfaces a server rejection inline", () => {
    render(<ConfigPanel settings={settings} meta={null} error="config file changed externally — restart the daemon or re-save from the CLI"
      disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/changed externally/)).toBeInTheDocument();
  });

  it("notes next-turn apply semantics", () => {
    render(<ConfigPanel settings={settings} meta={null} error={null} disabled={false} onSave={() => {}} />);
    expect(screen.getByText(/apply from the next turn/i)).toBeInTheDocument();
  });
});
```

- [ ] **Step 3: Run tests to verify they fail**

Run (in `web/`): `npm test -- --run ConfigPanel` → FAIL (placeholder has no form).

- [ ] **Step 4: Extract `SettingsForm`**

Create `web/src/components/SettingsForm.tsx` by MOVING from `SettingsPanel.tsx`: the `Meta` interface (rename to exported `SettingsMeta`), `toLines`/`fromLines`, all form state (`form`, `allow`, `deny`, `skillsDirs`, `set`, `toggleSkill`, `save`, `num`, `numVal`, `floor`, `redundant`, `field`, `label` constants) and ALL section JSX from the error banner through the Save button. Add ONE new section between "Model & inference" and "Command policy":

```tsx
<section className="mb-4 space-y-3">
  <h3 className="text-sm font-semibold text-[var(--text-strong)]">System prompt</h3>
  <div>
    <label className={label} htmlFor="system_prompt_override">Override (empty = built-in prompt)</label>
    <textarea id="system_prompt_override" rows={6} className={field}
      value={form.system_prompt_override ?? ""}
      onChange={(e) => set("system_prompt_override", e.target.value === "" ? null : e.target.value)} />
    <p className="mt-1 text-xs text-[var(--text-muted)]">
      Replaces the built-in base prompt; active skills still append on top.
    </p>
  </div>
</section>
```

`SettingsPanel.tsx` shrinks to the overlay shell:

```tsx
import type { RuntimeSettings } from "../wire";
import { SettingsForm, type SettingsMeta } from "./SettingsForm";

interface Props {
  settings: RuntimeSettings;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onClose: () => void;
}

export function SettingsPanel({ settings, meta, error, disabled, onSave, onClose }: Props) {
  return (
    <div className="absolute inset-0 z-10 flex justify-end" style={{ background: "rgba(0,0,0,0.5)" }}>
      <div className="h-full w-96 overflow-y-auto p-4 shadow-xl"
        style={{ background: "var(--surface-overlay)", color: "var(--text)" }}>
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-lg font-semibold" style={{ color: "var(--text-strong)" }}>Settings</h2>
          <button onClick={onClose} className="hover:opacity-80" style={{ color: "var(--text-muted)" }}>close</button>
        </div>
        <SettingsForm settings={settings} meta={meta} error={error} disabled={disabled} onSave={onSave} />
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Implement `ConfigPanel` over `SettingsForm`**

Replace `web/src/components/design/ConfigPanel.tsx`'s body:

```tsx
import type { RuntimeSettings } from "../../wire";
import { SettingsForm, type SettingsMeta } from "../SettingsForm";

export interface ConfigPanelProps {
  settings: RuntimeSettings | null;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
}

export function ConfigPanel({ settings, meta, error, disabled, onSave }: ConfigPanelProps) {
  if (!settings) {
    return <p className="p-4 text-sm" style={{ color: "var(--text-muted)" }}>Loading settings…</p>;
  }
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-4">
      <p className="mb-3 text-xs" style={{ color: "var(--text-muted)" }}>
        Changes apply from the next turn; an in-flight turn finishes on the old config.
      </p>
      {/* remount when fresh settings arrive so the form re-seeds */}
      <SettingsForm key={JSON.stringify(settings)} settings={settings} meta={meta}
        error={error} disabled={disabled} onSave={onSave} />
    </div>
  );
}
```

- [ ] **Step 6: Run tests to verify everything passes**

Run (in `web/`): `npm test -- --run` (FULL suite — the SettingsPanel extraction must keep `SettingsPanel.test.tsx` green) and `npm run typecheck`.

- [ ] **Step 7: Commit**

```bash
git add web/src/components/SettingsForm.tsx web/src/components/SettingsPanel.tsx web/src/components/design/ConfigPanel.tsx web/src/components/design/ConfigPanel.test.tsx web/src/wire.ts
git commit -m "feat(web): Config sub-section — SettingsForm extraction + system prompt override"
```

---

### Task 11: Full gate + manual end-to-end verification

**Files:** none (verification only; fix regressions if any).

- [ ] **Step 1: Run the CI gate**

From repo root: `bash scripts/ci.sh`
Expected: fmt + clippy + `cargo test` (agent/) + web typecheck/vitest all green. Fix anything red before proceeding.

- [ ] **Step 2: Manual e2e (desktop app)**

Launch: `npm run desktop:dev` (repo root). Then:

1. In chat, ask the agent: *"Use the render tool with id 'design:demo', kind html, title 'Demo', and a simple landing-page mockup. Then render a second variation with the same id."*
2. Open the **Design** tab → expect the Demo design at **v2 / 2**; step back to v1; expect the "v2 available" path when a third render arrives.
3. Verify the design does NOT appear in the Workspace tab's artifact tabs.
4. Click the mockup → draft pin appears; type a comment; **Send feedback** → the chat shows a user message containing a ` ```design-feedback ` block, and the pin flips to a sent marker.
5. Check `~/.agent/sessions/<id>.jsonl` contains that feedback turn.
6. Open Design → **Config**; set the System prompt override to something distinctive ("End every reply with DESIGN-MODE."); Save; send a chat message → the NEXT turn's reply reflects the override.
7. Edit the runtime config file on disk by hand, then Save again in Config → expect the inline "config file changed externally" error.

- [ ] **Step 3: Commit any fixes; then done**

Use `superpowers:finishing-a-development-branch` to decide merge/PR.

---

## Self-review (performed at plan-writing time)

- **Spec coverage:** third tab (T1), interception + versioning + N=20 + localStorage/SecurityError (T2), frozen feedback schema golden (T3), version bar/compare/badge/unsupported marker (T4), pins with pct coords + draft/sent lifecycle (T5), pane + App wiring + send path + Tauri gating (T6), render-tool teaching (T7), system prompt override end-to-end (T8+T10), external-edit refusal (T9), validation/inline errors ride the existing `settings_error` path (T9+T10 test), e2e incl. trace check (T11). Spec's `apply_live`/dead-session items are covered by the documented deviation (existing next-turn apply).
- **Type consistency:** `Pin {x_pct,y_pct,comment}` and 1-based `version` used identically in designStore/designFeedback/overlay/canvas/pane; `SettingsMeta` shape matches `state.ts` `settingsMeta`.
- **Placeholders:** none — every code step is complete; Task 8's "adapt helper names" instructs reading the module's existing fixtures, which is codebase-reading, not a design hole.
