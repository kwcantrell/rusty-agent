# Right-Pane Tab Restructure + Live-URL Canvas Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote the Config and Architecture subtabs to top-level right-pane tabs, and let the Design canvas render a live localhost dev server (agent `render kind=url` + manual URL field) so feedback annotates the real app instead of standalone HTML copies.

**Architecture:** Two independent parts. Part 1 (Tasks 1–3) is web-only: widen the `RightTab` union, render five tabs with Tauri gating in `RightPaneTabs`, strip `DesignPane` to pure canvas, and wire `ConfigPanel`/`ArchitecturePane` directly in `App`. Part 2 (Tasks 4–8) is end-to-end: a new `Display::Url` variant flows from the Rust `render` tool (localhost-only validation + tool-description steering) through `wire.ts` into a new `UrlArtifact` iframe, with an interact/pin toggle, a `url` field in the feedback payload, and a manual URL field backed by a new `designStore.addUrlVersion`.

**Tech Stack:** Rust (Cargo workspace under `agent/`, crate `agent-tools`), React 19 + TypeScript + Vitest + Testing Library (under `web/`).

**Spec:** `docs/superpowers/specs/2026-07-06-design-tab-restructure-url-canvas-design.md`

## Global Constraints

- Two Cargo workspaces exist; all Rust work here targets the `agent/` workspace (`cd agent` first; `source ~/.cargo/env` if `cargo` is missing).
- Web commands run from `web/` (`npx vitest run <file>` for single files, `npm test` for the suite).
- Conventional commits: `type(scope): summary`.
- URL policy (exact): scheme `http` or `https`; host exactly `localhost`, `127.0.0.1`, or `[::1]`; any port. Enforced in the Rust tool AND re-checked client-side. Fail closed.
- The `design-feedback` payload is a frozen contract: existing fields (`design_id`, `version`, `pins`, `note`) must not change; this plan only ADDS an optional `url` field.
- Manual-preview design id (exact string): `design:live-preview`.
- Do not touch sandbox networking or any screencast work — out of scope per spec.
- Work on a feature branch off `main` (created at execution time via superpowers:using-git-worktrees).

---

### Task 1: Widen `RightTab` storage with Tauri-aware fallback

**Files:**
- Modify: `web/src/storage.ts:81-91`
- Modify: `web/src/App.tsx:26` (call site — keeps typecheck green)
- Test: `web/src/storage.rightTab.test.ts`

**Interfaces:**
- Produces: `type RightTab = "workspace" | "context" | "design" | "architecture" | "config"`; `loadRightTab(tauri: boolean): RightTab`; `saveRightTab(t: RightTab): void` (unchanged signature).
- Consumed by: Task 2 (`RightPaneTabs` labels), Task 3 (`App` tab branches).

- [ ] **Step 1: Rewrite the storage test for the widened union**

Replace the whole body of `web/src/storage.rightTab.test.ts` with:

```ts
import { describe, it, expect, beforeEach } from "vitest";
import { loadRightTab, saveRightTab } from "./storage";

describe("right tab persistence", () => {
  beforeEach(() => localStorage.clear());

  it("defaults to workspace", () => {
    expect(loadRightTab(true)).toBe("workspace");
  });

  it("round-trips design", () => {
    saveRightTab("design");
    expect(loadRightTab(false)).toBe("design");
  });

  it("round-trips context", () => {
    saveRightTab("context");
    expect(loadRightTab(false)).toBe("context");
  });

  it("round-trips architecture and config under Tauri", () => {
    saveRightTab("architecture");
    expect(loadRightTab(true)).toBe("architecture");
    saveRightTab("config");
    expect(loadRightTab(true)).toBe("config");
  });

  it("falls back to workspace for Tauri-only tabs outside Tauri", () => {
    saveRightTab("architecture");
    expect(loadRightTab(false)).toBe("workspace");
    saveRightTab("config");
    expect(loadRightTab(false)).toBe("workspace");
  });

  it("falls back to workspace on a stale stored value", () => {
    localStorage.setItem("rightTab", "garbage");
    expect(loadRightTab(true)).toBe("workspace");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/storage.rightTab.test.ts`
Expected: FAIL — type errors / wrong values (`loadRightTab` takes no argument; `"architecture"` is not a `RightTab`).

- [ ] **Step 3: Implement the widened type and fallback**

In `web/src/storage.ts`, replace lines 81–91 (the `RIGHT_TAB` block) with:

```ts
const RIGHT_TAB = "rightTab";
export type RightTab = "workspace" | "context" | "design" | "architecture" | "config";
const ALL_TABS: readonly RightTab[] = ["workspace", "context", "design", "architecture", "config"];
const TAURI_ONLY: readonly RightTab[] = ["architecture", "config"];

export function loadRightTab(tauri: boolean): RightTab {
  try {
    const v = localStorage.getItem(RIGHT_TAB) as RightTab | null;
    if (!v || !ALL_TABS.includes(v)) return "workspace";
    return !tauri && TAURI_ONLY.includes(v) ? "workspace" : v;
  } catch { return "workspace"; }
}
export function saveRightTab(t: RightTab): void {
  try { localStorage.setItem(RIGHT_TAB, t); } catch { /* ignore */ }
}
```

In `web/src/App.tsx`, change line 26 from:

```tsx
  const [rightTab, setRightTab] = useState<RightTab>(loadRightTab);
```

to:

```tsx
  const [rightTab, setRightTab] = useState<RightTab>(() => loadRightTab(isTauri()));
```

(`isTauri` is already imported in `App.tsx` line 3.)

- [ ] **Step 4: Run the test and the typecheck to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/storage.rightTab.test.ts && npm run typecheck`
Expected: storage test PASS. Typecheck FAILS only if other callers of `loadRightTab` exist — there are none besides `App.tsx`. `RightPaneTabs.tsx` still compiles because its `LABELS: Record<RightTab, string>` is now missing keys — if typecheck reports that, it is expected and fixed in Task 2; in that case run only the vitest half here and rely on Task 2's typecheck.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/storage.ts web/src/storage.rightTab.test.ts web/src/App.tsx
git commit -m "feat(web): widen RightTab to architecture/config with tauri-aware fallback"
```

---

### Task 2: Five top-level tabs in `RightPaneTabs` (Tauri-gated)

**Files:**
- Modify: `web/src/components/RightPaneTabs.tsx`
- Test: `web/src/components/RightPaneTabs.test.tsx`

**Interfaces:**
- Consumes: `RightTab`, `saveRightTab` from Task 1.
- Produces: `RightPaneTabs({ rightTab, setRightTab })` — same props, now renders Workspace | Context | Design always, plus Architecture | Config when `isTauri()`.

- [ ] **Step 1: Rewrite the component test with a Tauri mock**

Replace the whole body of `web/src/components/RightPaneTabs.test.tsx` with (Tauri-mock pattern copied from `DesignPane.test.tsx`):

```tsx
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";

const tauriMock = vi.hoisted(() => ({ value: true }));
vi.mock("../transport", () => ({ isTauri: () => tauriMock.value }));

import { RightPaneTabs } from "./RightPaneTabs";

describe("RightPaneTabs", () => {
  beforeEach(() => { tauriMock.value = true; });

  it("renders all five tabs under Tauri", () => {
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    for (const name of ["Workspace", "Context", "Design", "Architecture", "Config"]) {
      expect(screen.getByRole("tab", { name })).toBeInTheDocument();
    }
  });

  it("hides Architecture and Config outside Tauri", () => {
    tauriMock.value = false;
    render(<RightPaneTabs rightTab="workspace" setRightTab={() => {}} />);
    expect(screen.getByRole("tab", { name: "Design" })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Architecture" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Config" })).not.toBeInTheDocument();
  });

  it("selects Config on click", () => {
    const picked: string[] = [];
    render(<RightPaneTabs rightTab="workspace" setRightTab={(t) => picked.push(t)} />);
    fireEvent.click(screen.getByRole("tab", { name: "Config" }));
    expect(picked).toEqual(["config"]);
    expect(screen.getByRole("tab", { name: "Workspace" })).toHaveAttribute("aria-selected", "true");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/RightPaneTabs.test.tsx`
Expected: FAIL — no tab named "Architecture" / "Config".

- [ ] **Step 3: Implement the five-tab strip**

Replace the whole body of `web/src/components/RightPaneTabs.tsx` with:

```tsx
import type { RightTab } from "../storage";
import { saveRightTab } from "../storage";
import { isTauri } from "../transport";

const LABELS: Record<RightTab, string> = {
  workspace: "Workspace", context: "Context", design: "Design",
  architecture: "Architecture", config: "Config",
};
const BASE: readonly RightTab[] = ["workspace", "context", "design"];
const TAURI_TABS: readonly RightTab[] = ["architecture", "config"];

export function RightPaneTabs(
  { rightTab, setRightTab }: { rightTab: RightTab; setRightTab: (t: RightTab) => void },
) {
  const tabs = isTauri() ? [...BASE, ...TAURI_TABS] : BASE;
  return (
    <div className="flex gap-1 px-2 pt-2" role="tablist" style={{ borderBottom: "1px solid var(--border)" }}>
      {tabs.map((t) => (
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

- [ ] **Step 4: Run the test and typecheck to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/RightPaneTabs.test.tsx && npm run typecheck`
Expected: PASS, and the Task-1 `LABELS` type error (if it appeared) is now gone.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/RightPaneTabs.tsx web/src/components/RightPaneTabs.test.tsx
git commit -m "feat(web): promote Architecture and Config to top-level right-pane tabs"
```

---

### Task 3: Strip `DesignPane` to pure canvas; wire panes in `App`

**Files:**
- Modify: `web/src/components/design/DesignPane.tsx`
- Modify: `web/src/components/design/ConfigPanel.tsx`
- Modify: `web/src/App.tsx:139-154` (rightPane block) and line 16 area (imports)
- Test: `web/src/components/design/DesignPane.test.tsx`
- Test: `web/src/components/design/ConfigPanel.test.tsx`

**Interfaces:**
- Produces: `DesignPaneProps` shrinks to `{ items: Item[]; sessionId: string; onSend: (text: string) => void; sendDisabled: boolean }`. `ConfigPanelProps` gains optional `onLoad?: () => void`, called once on mount (this replaces the old subtab-click `onLoadSettings` trigger — the spec's "Config tab triggers settings_get" test lands here at component level).
- Consumes: `RightTab` branches from Tasks 1–2.

- [ ] **Step 1: Update the tests**

In `web/src/components/design/ConfigPanel.test.tsx`, add inside the existing `describe("ConfigPanel", ...)`:

```tsx
  it("calls onLoad once on mount to fetch fresh settings", () => {
    const onLoad = vi.fn();
    const { rerender } = render(<ConfigPanel settings={null} meta={null} error={null}
      disabled={false} onSave={() => {}} onLoad={onLoad} />);
    rerender(<ConfigPanel settings={settings} meta={null} error={null}
      disabled={false} onSave={() => {}} onLoad={onLoad} />);
    expect(onLoad).toHaveBeenCalledTimes(1);
  });
```

and extend the vitest import at the top to include `vi`:

```tsx
import { describe, it, expect, vi } from "vitest";
```

Replace the whole body of `web/src/components/design/DesignPane.test.tsx` with (subtab tests deleted; settings props and architecture/tauri mocks gone):

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import type { Item } from "../../state";
import { DesignPane } from "./DesignPane";

const designItem = (html: string): Item =>
  ({ kind: "tool", name: "render", args: {}, status: "done",
     display: { Html: { html, id: "design:landing", title: "Landing" } } });

const base = { sessionId: "s1", onSend: () => {}, sendDisabled: false };

describe("DesignPane", () => {
  beforeEach(() => { localStorage.clear(); });

  it("shows an empty state with no designs", () => {
    render(<DesignPane {...base} items={[]} />);
    expect(screen.getByText(/No designs yet/)).toBeInTheDocument();
  });

  it("renders the latest design version in the canvas", () => {
    render(<DesignPane {...base} items={[designItem("<p>v1</p>"), designItem("<p>v2</p>")]} />);
    expect(screen.getByText("v2 / 2")).toBeInTheDocument();
  });

  it("has no Config or Architecture sub-tabs", () => {
    render(<DesignPane {...base} items={[]} />);
    expect(screen.queryByRole("tab", { name: "Config" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Architecture" })).not.toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Canvas" })).not.toBeInTheDocument();
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
});
```

- [ ] **Step 2: Run both tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/design/DesignPane.test.tsx src/components/design/ConfigPanel.test.tsx`
Expected: FAIL — `ConfigPanel` has no `onLoad` prop; `DesignPane` still renders a "Canvas" tab (and its props type still requires settings props, a TS error).

- [ ] **Step 3: Implement**

Replace the whole body of `web/src/components/design/DesignPane.tsx` with:

```tsx
import { useState } from "react";
import type { Item } from "../../state";
import { useDesignStore } from "../../designStore";
import { buildFeedbackMessage } from "../../designFeedback";
import { DesignCanvas } from "./DesignCanvas";

export interface DesignPaneProps {
  items: Item[];
  sessionId: string;
  onSend: (text: string) => void;
  sendDisabled: boolean;
}

export function DesignPane({ items, sessionId, onSend, sendDisabled }: DesignPaneProps) {
  const store = useDesignStore(items, sessionId);
  const [activeId, setActiveId] = useState<string | null>(null);
  const active = store.designs.find((d) => d.id === activeId) ?? store.designs[store.designs.length - 1];
  const sub = (on: boolean) => ({
    color: on ? "var(--text-strong)" : "var(--text-muted)", fontWeight: on ? 600 : 400,
  });

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      {!active ? (
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

In `web/src/components/design/ConfigPanel.tsx`, replace the whole body with:

```tsx
import { useEffect } from "react";
import type { RuntimeSettings } from "../../wire";
import { SettingsForm, type SettingsMeta } from "../SettingsForm";

export interface ConfigPanelProps {
  settings: RuntimeSettings | null;
  meta: SettingsMeta | null;
  error: string | null;
  disabled: boolean;
  onSave: (s: RuntimeSettings) => void;
  onLoad?: () => void;
}

export function ConfigPanel({ settings, meta, error, disabled, onSave, onLoad }: ConfigPanelProps) {
  // Fetch fresh settings once when the panel opens (was the subtab's click handler).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { onLoad?.(); }, []);
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

In `web/src/App.tsx`:

1. Add two imports after line 16 (`import { DesignPane } ...`):

```tsx
import { ConfigPanel } from "./components/design/ConfigPanel";
import { ArchitecturePane } from "./components/design/ArchitecturePane";
```

2. Replace the `rightPane` block (lines 139–154) with:

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
            : rightTab === "architecture"
              ? <ArchitecturePane />
              : rightTab === "config"
                ? <ConfigPanel settings={state.settings} meta={state.settingsMeta} error={state.settingsError}
                    disabled={!connected} onSave={saveSettings}
                    onLoad={() => sock.current?.send({ kind: "settings_get" })} />
                : <DesignPane items={state.items} sessionId={sessionId} onSend={send} sendDisabled={!connected} />}
      </div>
    </div>
  );
```

Note: `ConfigPanel` is rendered in a `flex` column whose child needs height — it already handles its own scroll (`overflow-y-auto`). The `rightTab === "config"` branch only renders under Tauri (the tab isn't offered otherwise), matching the old subtab gating.

- [ ] **Step 4: Run the design tests, full web suite, and typecheck**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/design/ && npm test && npm run typecheck`
Expected: PASS (`App.tauri.test.tsx` never references the design tab or its props, so the whole suite stays green).

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/design/DesignPane.tsx web/src/components/design/DesignPane.test.tsx \
  web/src/components/design/ConfigPanel.tsx web/src/components/design/ConfigPanel.test.tsx web/src/App.tsx
git commit -m "feat(web): DesignPane is pure canvas; Config/Architecture render from App tabs"
```

---

### Task 4: Rust `Display::Url` + `render kind=url` with localhost guard and steering

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs:43-102` (add variant after `Image`)
- Modify: `agent/crates/agent-tools/src/render.rs`
- Test: inline `#[cfg(test)]` in `agent/crates/agent-tools/src/render.rs`

**Interfaces:**
- Produces: `Display::Url { url: String, title: Option<String>, id: Option<String> }` (serde external-tag serialization → wire JSON `{"Url":{"url":...,"title":...,"id":...}}`); `render` tool accepts `kind:"url"` with `content` = the URL.
- Consumed by: Task 5 (`wire.ts` mirror).

- [ ] **Step 1: Write the failing tests**

In `agent/crates/agent-tools/src/render.rs`, add to the `mod tests` block:

```rust
    #[tokio::test]
    async fn render_url_localhost_emits_url_display() {
        let out = RenderArtifact
            .execute(
                json!({"kind":"url","title":"App","content":"http://localhost:5173/app"}),
                &ctx(),
            )
            .await
            .unwrap();
        match out.display {
            Some(Display::Url { url, title, .. }) => {
                assert_eq!(url, "http://localhost:5173/app");
                assert_eq!(title.as_deref(), Some("App"));
            }
            other => panic!("expected Url, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_url_accepts_all_loopback_hosts() {
        for u in [
            "http://localhost:5173",
            "https://localhost/",
            "http://127.0.0.1:3000/x?y=1",
            "http://[::1]:8080/x",
            "http://LOCALHOST:80",
        ] {
            RenderArtifact
                .execute(json!({"kind":"url","content":u}), &ctx())
                .await
                .unwrap_or_else(|e| panic!("{u} should be accepted: {e:?}"));
        }
    }

    #[tokio::test]
    async fn render_url_rejects_non_local_targets() {
        for u in [
            "http://evil.com",
            "http://localhost.evil.com:5173",
            "http://localhost@evil.com/",
            "http://user@localhost:5173/",
            "ftp://localhost/",
            "localhost:5173",
            "http://[::1/x",
        ] {
            let err = RenderArtifact
                .execute(json!({"kind":"url","content":u}), &ctx())
                .await
                .expect_err(&format!("{u} should be rejected"));
            assert!(matches!(err, ToolError::InvalidArgs(_)));
        }
    }

    #[test]
    fn description_steers_url_over_standalone_html() {
        let t = RenderArtifact;
        assert!(
            t.description().contains("kind=url") && t.description().contains("dev server"),
            "agents must learn to prefer the live dev server over standalone html"
        );
        let kinds = t.schema().parameters["properties"]["kind"]["enum"]
            .as_array()
            .unwrap()
            .clone();
        assert!(kinds.iter().any(|k| k == "url"));
    }
```

Note: `http://user@localhost:5173/` is rejected on purpose — the guard exact-matches the authority-up-to-port, so userinfo tricks fail closed.

- [ ] **Step 2: Run tests to verify they fail**

Run: `source ~/.cargo/env; cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-tools render`
Expected: FAIL — `Display::Url` does not exist / `unknown kind \`url\``.

- [ ] **Step 3: Implement**

In `agent/crates/agent-tools/src/types.rs`, add a variant to `enum Display` after the `Image { ... }` variant:

```rust
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
```

In `agent/crates/agent-tools/src/render.rs`:

1. Replace the `description` method with:

```rust
    fn description(&self) -> &str {
        "Render an artifact (markdown, code, html, mermaid diagram, table, image, or a live \
         localhost url) into the user's Inspector panel. When a dev server is already running \
         (e.g. Vite), prefer `kind=url` with its address (content=\"http://localhost:5173\") so \
         the user sees the real app and their feedback maps to the actual code; use `kind=html` \
         only for one-off static mockups when no dev server exists. For iterative visual design, \
         use an id starting with `design:` (e.g. `design:landing-page`): each re-render of that \
         id adds a new version to the user's Design canvas, where they can step through versions, \
         compare them, and pin feedback that comes back to you as a `design-feedback` message."
    }
```

2. In `schema()`, change the `kind` enum line to:

```rust
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image","url"],
                        "description": "Which artifact kind to render; one of the allowed enum values."},
```

and the `content` description to:

```rust
                    "content": {"type": "string",
                        "description": "primary payload: markdown/html/mermaid source, code text, base64 image data, or the localhost dev-server address (kind=url)"},
```

3. Add the validator as a free function next to `str_arg`/`opt_str`:

```rust
/// `kind=url` targets must be the user's own dev server: http(s) with host exactly
/// `localhost`, `127.0.0.1`, or `[::1]` (any port). Exact-matching the authority up
/// to the port fails closed on userinfo tricks (`http://localhost@evil.com`).
fn validate_local_url(url: &str) -> Result<(), ToolError> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| ToolError::InvalidArgs(format!("url must be http(s): `{url}`")))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        match authority.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => format!("{h}]"),
            _ => return Err(ToolError::InvalidArgs(format!("malformed url: `{url}`"))),
        }
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    match host.to_ascii_lowercase().as_str() {
        "localhost" | "127.0.0.1" | "[::1]" => Ok(()),
        other => Err(ToolError::InvalidArgs(format!(
            "url host must be localhost, 127.0.0.1, or [::1] — got `{other}`"
        ))),
    }
}
```

4. Add the match arm in `execute` before the `other =>` arm:

```rust
            "url" => {
                let url = str_arg(&args, "content")?;
                validate_local_url(&url)?;
                Display::Url {
                    url,
                    title: title.clone(),
                    id,
                }
            }
```

- [ ] **Step 4: Run tests, then build the whole workspace**

Run: `source ~/.cargo/env; cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-tools && cargo build && cargo clippy --all-targets -- -D warnings && cargo fmt --check`
Expected: all PASS. (`agent-cli`'s `render.rs` uses `if let` on `Display`, so the new variant is non-breaking; a full build confirms no exhaustive match elsewhere.)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-tools/src/types.rs agent/crates/agent-tools/src/render.rs
git commit -m "feat(tools): render kind=url with localhost-only guard; steer agents to the live dev server"
```

---

### Task 5: Web `Url` display — wire type, renderable set, guard, `UrlArtifact`

**Files:**
- Modify: `web/src/wire.ts:3-12` (Display union)
- Modify: `web/src/designStore.ts:12` (RENDERABLE set)
- Create: `web/src/components/inspector/urlGuard.ts`
- Create: `web/src/components/inspector/UrlArtifact.tsx`
- Modify: `web/src/components/inspector/ArtifactRenderer.tsx`
- Test: `web/src/components/inspector/urlGuard.test.ts` (create)
- Test: `web/src/components/inspector/UrlArtifact.test.tsx` (create)
- Test: `web/src/designStore.test.ts` (add one case)

**Interfaces:**
- Consumes: wire shape `{ Url: { url: string; title?: string; id?: string } }` from Task 4.
- Produces: `isLocalUrl(raw: string): boolean`; `isMixedContent(raw: string, pageProtocol?: string): boolean`; `UrlArtifact({ url }: { url: string })`; `ArtifactRenderer` handles `"Url" in display`. Tasks 6–8 rely on `"Url" in display` narrowing and `isLocalUrl`.

- [ ] **Step 1: Write the failing tests**

Create `web/src/components/inspector/urlGuard.test.ts`:

```ts
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
```

Create `web/src/components/inspector/UrlArtifact.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { UrlArtifact } from "./UrlArtifact";

describe("UrlArtifact", () => {
  it("renders a live iframe for a localhost url", () => {
    render(<UrlArtifact url="http://localhost:5173/app" />);
    const frame = screen.getByTitle("live preview");
    expect(frame).toHaveAttribute("src", "http://localhost:5173/app");
    expect(frame).toHaveAttribute("sandbox", "allow-scripts allow-same-origin");
  });

  it("blocks non-localhost urls with a notice instead of an iframe", () => {
    render(<UrlArtifact url="http://evil.com/" />);
    expect(screen.queryByTitle("live preview")).not.toBeInTheDocument();
    expect(screen.getByText(/only localhost/i)).toBeInTheDocument();
  });
});
```

(The mixed-content branch is unreachable in jsdom, whose page protocol is `http:` — it is covered by the `isMixedContent` unit test above.)

In `web/src/designStore.test.ts`, add inside `describe("designsFrom", ...)`:

```ts
  it("treats Url displays as renderable design versions", () => {
    const [d] = designsFrom([toolItem({ Url: { url: "http://localhost:5173", id: "design:app" } })]);
    expect(d.id).toBe("design:app");
    expect(d.versions[0].renderable).toBe(true);
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/inspector/ src/designStore.test.ts`
Expected: FAIL — `urlGuard` / `UrlArtifact` modules don't exist; `Url` not assignable to `Display`; renderable is `false`.

- [ ] **Step 3: Implement**

In `web/src/wire.ts`, add to the `Display` union after the `Image` line:

```ts
  | { Url: { url: string; title?: string; id?: string } };
```

(and change the previous last member's `;` to nothing — the union's semicolon moves to the new last line).

In `web/src/designStore.ts`, line 12, add `"Url"`:

```ts
const RENDERABLE = new Set(["Text", "Markdown", "Code", "Diff", "Terminal", "Table", "Image", "Html", "Mermaid", "Url"]);
```

Create `web/src/components/inspector/urlGuard.ts`:

```ts
/** True only for http(s) URLs whose host is the local machine. Mirrors the Rust
 *  tool-side guard (render.rs validate_local_url); both must hold — fail closed. */
export function isLocalUrl(raw: string): boolean {
  try {
    const u = new URL(raw);
    const local = ["localhost", "127.0.0.1", "[::1]", "::1"];
    return (u.protocol === "http:" || u.protocol === "https:") && local.includes(u.hostname);
  } catch { return false; }
}

/** An https-served page cannot embed an http iframe (browser mixed-content block). */
export function isMixedContent(raw: string, pageProtocol: string = window.location.protocol): boolean {
  try { return pageProtocol === "https:" && new URL(raw).protocol === "http:"; } catch { return true; }
}
```

Create `web/src/components/inspector/UrlArtifact.tsx`:

```tsx
import { isLocalUrl, isMixedContent } from "./urlGuard";

function Notice({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center p-6 text-center text-sm"
      style={{ color: "var(--text-muted)", minHeight: "240px" }}>
      <p>{text}</p>
    </div>
  );
}

// Live preview of the user's own dev server. Unlike agent-authored HTML (fully
// sandboxed), a real app needs scripts and its own origin — acceptable only
// because non-localhost targets never reach the iframe.
export function UrlArtifact({ url }: { url: string }) {
  if (!isLocalUrl(url)) {
    return <Notice text={`Blocked: only localhost URLs render here (got ${url}).`} />;
  }
  if (isMixedContent(url)) {
    return <Notice text={"This page is served over HTTPS, so the browser blocks embedding an "
      + "http:// localhost app. Use the desktop app (or a locally served UI) for live preview."} />;
  }
  return (
    <iframe title="live preview" src={url} sandbox="allow-scripts allow-same-origin"
      className="h-full w-full"
      style={{ border: "none", minHeight: "240px", background: "var(--surface-overlay)" }} />
  );
}
```

In `web/src/components/inspector/ArtifactRenderer.tsx`, add an import and a branch after the `Html` branch:

```tsx
import { UrlArtifact } from "./UrlArtifact";
```

```tsx
  if ("Url" in display) {
    return <UrlArtifact url={display.Url.url} />;
  }
```

- [ ] **Step 4: Run the tests and typecheck to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/inspector/ src/designStore.test.ts && npm run typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/wire.ts web/src/designStore.ts web/src/designStore.test.ts \
  web/src/components/inspector/urlGuard.ts web/src/components/inspector/urlGuard.test.ts \
  web/src/components/inspector/UrlArtifact.tsx web/src/components/inspector/UrlArtifact.test.tsx \
  web/src/components/inspector/ArtifactRenderer.tsx
git commit -m "feat(web): render Url displays as a guarded live-preview iframe"
```

---

### Task 6: Interact / Pin-feedback toggle for URL versions

**Files:**
- Modify: `web/src/components/design/AnnotationOverlay.tsx`
- Modify: `web/src/components/design/DesignCanvas.tsx`
- Test: `web/src/components/design/DesignCanvas.test.tsx` (add cases)
- Test: `web/src/components/design/AnnotationOverlay.test.tsx` (add case)

**Interfaces:**
- Produces: `AnnotationOverlay` gains `passthrough?: boolean` (default `false`) — when true the pin layer sets `pointer-events: none` so the iframe underneath is interactive. `DesignCanvas` shows an "Interact" / "Pin feedback" toggle (buttons with `aria-pressed`) only when the current version is a `Url` display; pin mode is the default.
- Consumes: `Url` display narrowing from Task 5.

- [ ] **Step 1: Write the failing tests**

In `web/src/components/design/AnnotationOverlay.test.tsx`, add inside the top-level `describe`:

```tsx
  it("lets clicks pass through to the artifact when passthrough is set", () => {
    render(<AnnotationOverlay sent={[]} disabled={false} onSend={() => {}} passthrough>
      <div>content</div>
    </AnnotationOverlay>);
    expect(screen.getByTestId("pin-layer")).toHaveStyle({ pointerEvents: "none" });
  });
```

In `web/src/components/design/DesignCanvas.test.tsx`, add a fixture near `design(n)` and new cases:

```tsx
const urlDesign = (): Design => ({
  id: "design:app", title: "App",
  versions: [{ display: { Url: { url: "http://localhost:5173", id: "design:app" } }, renderable: true }],
});
```

```tsx
  it("offers an interact/pin toggle only for url versions", () => {
    render(<DesignCanvas design={urlDesign()} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.getByRole("button", { name: "Interact" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pin feedback" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByTestId("pin-layer")).not.toHaveStyle({ pointerEvents: "none" });
  });

  it("interact mode disables the pin layer so the live app is usable", () => {
    render(<DesignCanvas design={urlDesign()} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    fireEvent.click(screen.getByRole("button", { name: "Interact" }));
    expect(screen.getByTestId("pin-layer")).toHaveStyle({ pointerEvents: "none" });
  });

  it("html versions get no toggle", () => {
    render(<DesignCanvas design={design(1)} sentPins={noPins}
      onSendFeedback={() => {}} sendDisabled={false} />);
    expect(screen.queryByRole("button", { name: "Interact" })).not.toBeInTheDocument();
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/design/DesignCanvas.test.tsx src/components/design/AnnotationOverlay.test.tsx`
Expected: FAIL — no `passthrough` prop, no "Interact" button.

- [ ] **Step 3: Implement**

In `web/src/components/design/AnnotationOverlay.tsx`, change the signature and pin-layer div:

```tsx
export function AnnotationOverlay({ children, sent, disabled, onSend, passthrough = false }: {
  children: ReactNode; sent: Pin[]; disabled: boolean; onSend: (pins: Pin[]) => void;
  passthrough?: boolean;
}) {
```

```tsx
        <div data-testid="pin-layer" className="absolute inset-0 cursor-crosshair" onClick={addPin}
          style={passthrough ? { pointerEvents: "none" } : undefined}>
```

In `web/src/components/design/DesignCanvas.tsx`, replace the whole body with:

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
  const [interact, setInteract] = useState(false);
  const total = design.versions.length;
  const cur = Math.min(viewed ?? total - 1, total - 1);
  const behind = viewed !== null && cur < total - 1;
  const curDisplay = design.versions[cur].display;
  const liveUrl = "Url" in curDisplay ? curDisplay.Url.url : null;
  const modeBtn = (on: boolean) => ({
    background: on ? "var(--accent)" : "transparent",
    color: on ? "var(--accent-fg)" : "var(--text-muted)",
    border: "1px solid var(--border)",
  });
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
      {liveUrl && !compare && (
        <div className="flex gap-1 px-3 pt-2" role="group" aria-label="canvas mode">
          <button aria-pressed={interact} onClick={() => setInteract(true)}
            className="rounded px-2 py-0.5 text-xs" style={modeBtn(interact)}>Interact</button>
          <button aria-pressed={!interact} onClick={() => setInteract(false)}
            className="rounded px-2 py-0.5 text-xs" style={modeBtn(!interact)}>Pin feedback</button>
        </div>
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
            passthrough={!!liveUrl && interact}
            onSend={(pins) => onSendFeedback(cur + 1, pins)}>
            <ArtifactRenderer display={design.versions[cur].display} />
          </AnnotationOverlay>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the design tests to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/components/design/ && npm run typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/design/AnnotationOverlay.tsx web/src/components/design/AnnotationOverlay.test.tsx \
  web/src/components/design/DesignCanvas.tsx web/src/components/design/DesignCanvas.test.tsx
git commit -m "feat(web): interact/pin toggle so live url previews stay usable under the pin layer"
```

---

### Task 7: Optional `url` field in the feedback payload

**Files:**
- Modify: `web/src/designFeedback.ts`
- Modify: `web/src/components/design/DesignCanvas.tsx` (pass url through `onSendFeedback`)
- Modify: `web/src/components/design/DesignPane.tsx` (accept url, forward to builder)
- Test: `web/src/designFeedback.test.ts` (add golden case)

**Interfaces:**
- Produces: `buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string, url?: string): string` — `url` appears in the JSON payload after `note`, only when defined. `DesignCanvas`'s `onSendFeedback` prop becomes `(version: number, pins: Pin[], url?: string) => void` and passes the current version's live URL when it is a `Url` display.
- Consumes: `liveUrl` narrowing from Task 6.

- [ ] **Step 1: Write the failing golden test**

In `web/src/designFeedback.test.ts`, add:

```ts
  it("includes the live url for url-version feedback (frozen-contract extension)", () => {
    const msg = buildFeedbackMessage("design:app", 2,
      [{ x_pct: 0.1, y_pct: 0.9, comment: "nav overlaps logo" }],
      undefined, "http://localhost:5173/settings");
    expect(msg).toBe(`Design feedback on design:app (v2):

\`\`\`design-feedback
{
  "design_id": "design:app",
  "version": 2,
  "pins": [
    {
      "x_pct": 0.1,
      "y_pct": 0.9,
      "comment": "nav overlaps logo"
    }
  ],
  "url": "http://localhost:5173/settings"
}
\`\`\``);
  });

  it("omits url when absent (existing golden unchanged)", () => {
    const msg = buildFeedbackMessage("design:x", 1, [{ x_pct: 0.5, y_pct: 0.5, comment: "c" }]);
    expect(msg).not.toContain('"url"');
  });
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/designFeedback.test.ts`
Expected: FAIL — 5th argument not accepted / no `url` in payload.

- [ ] **Step 3: Implement**

Replace `web/src/designFeedback.ts` with:

```ts
import type { Pin } from "./designStore";

/**
 * FROZEN CONTRACT (B-migration): this JSON shape becomes the DesignFeedback
 * tool-result payload when the first-class design channel lands. Existing field
 * names and structure must not change — the golden tests pin the exact output.
 * `url` (optional) identifies the live page a url-version's pins refer to;
 * pins on live apps are viewport-relative (iframe scroll is invisible cross-origin).
 */
export function buildFeedbackMessage(designId: string, version: number, pins: Pin[], note?: string, url?: string): string {
  const payload: Record<string, unknown> = { design_id: designId, version, pins };
  if (note !== undefined && note.trim().length > 0) payload.note = note;
  if (url !== undefined) payload.url = url;
  return `Design feedback on ${designId} (v${version}):\n\n\`\`\`design-feedback\n${JSON.stringify(payload, null, 2)}\n\`\`\``;
}
```

In `web/src/components/design/DesignCanvas.tsx`, change the prop type and the overlay's send callback:

```tsx
  onSendFeedback: (version: number, pins: Pin[], url?: string) => void;
```

```tsx
            onSend={(pins) => onSendFeedback(cur + 1, pins, liveUrl ?? undefined)}>
```

In `web/src/components/design/DesignPane.tsx`, change the `DesignCanvas` callback:

```tsx
            onSendFeedback={(v, pins, url) => {
              onSend(buildFeedbackMessage(active.id, v, pins, undefined, url));
              store.recordSent(active.id, v, pins);
            }}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/designFeedback.test.ts src/components/design/ && npm run typecheck`
Expected: PASS (existing goldens untouched).

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/designFeedback.ts web/src/designFeedback.test.ts \
  web/src/components/design/DesignCanvas.tsx web/src/components/design/DesignPane.tsx
git commit -m "feat(web): design feedback carries the live url for url-version pins"
```

---

### Task 8: Manual URL field backed by `designStore.addUrlVersion`

**Files:**
- Modify: `web/src/designStore.ts`
- Modify: `web/src/components/design/DesignPane.tsx`
- Test: `web/src/designStore.test.ts` (hook cases)
- Test: `web/src/components/design/DesignPane.test.tsx` (UI cases)

**Interfaces:**
- Consumes: `isLocalUrl` from Task 5 (`../inspector/urlGuard` relative to DesignPane).
- Produces: `export const LIVE_PREVIEW_ID = "design:live-preview"` and `DesignStoreApi.addUrlVersion(url: string): void` — appends a `{ Url: { url, id: LIVE_PREVIEW_ID, title: "Live preview" } }` version to the `design:live-preview` design (creating it if absent), capped at `MAX_VERSIONS`, persisted to localStorage like any other design.

- [ ] **Step 1: Write the failing tests**

In `web/src/designStore.test.ts`, add a new describe block (note: `renderHook`/`act` are already imported):

```ts
describe("useDesignStore.addUrlVersion", () => {
  beforeEach(() => localStorage.clear());

  it("creates the live-preview design and appends versions", () => {
    const { result } = renderHook(() => useDesignStore([], "s1"));
    act(() => result.current.addUrlVersion("http://localhost:5173"));
    act(() => result.current.addUrlVersion("http://localhost:5173/settings"));
    const d = result.current.designs.find((x) => x.id === LIVE_PREVIEW_ID);
    expect(d).toBeDefined();
    expect(d!.title).toBe("Live preview");
    expect(d!.versions).toHaveLength(2);
    expect((d!.versions[1].display as { Url: { url: string } }).Url.url)
      .toBe("http://localhost:5173/settings");
  });

  it("persists manual versions across remounts", () => {
    const first = renderHook(() => useDesignStore([], "s1"));
    act(() => first.result.current.addUrlVersion("http://localhost:5173"));
    first.unmount();
    const second = renderHook(() => useDesignStore([], "s1"));
    const d = second.result.current.designs.find((x) => x.id === LIVE_PREVIEW_ID);
    expect(d?.versions).toHaveLength(1);
  });
});
```

and extend the designStore import to include `LIVE_PREVIEW_ID`:

```ts
import {
  displayDesignId, designsFrom, mergeDesigns, useDesignStore, MAX_VERSIONS, LIVE_PREVIEW_ID,
} from "./designStore";
```

In `web/src/components/design/DesignPane.test.tsx`, add:

```tsx
  it("previews a manually entered localhost url on the canvas", () => {
    render(<DesignPane {...base} items={[]} />);
    fireEvent.change(screen.getByLabelText("preview url"), {
      target: { value: "http://localhost:5173" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.getByTitle("live preview")).toHaveAttribute("src", "http://localhost:5173");
  });

  it("rejects a non-localhost manual url with an inline error", () => {
    render(<DesignPane {...base} items={[]} />);
    fireEvent.change(screen.getByLabelText("preview url"), {
      target: { value: "http://evil.com" } });
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(screen.queryByTitle("live preview")).not.toBeInTheDocument();
    expect(screen.getByText(/Only localhost URLs/)).toBeInTheDocument();
  });
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/designStore.test.ts src/components/design/DesignPane.test.tsx`
Expected: FAIL — `LIVE_PREVIEW_ID` / `addUrlVersion` don't exist; no "preview url" input.

- [ ] **Step 3: Implement**

In `web/src/designStore.ts`:

1. Add after `MAX_VERSIONS`:

```ts
export const LIVE_PREVIEW_ID = "design:live-preview";
```

2. Extend the API type:

```ts
export interface DesignStoreApi {
  designs: Design[];
  sentPins: (designId: string, version: number) => Pin[];
  recordSent: (designId: string, version: number, pins: Pin[]) => void;
  addUrlVersion: (url: string) => void;
}
```

3. Replace the `useDesignStore` hook body with (stored blob becomes state so manual
   versions can append; it stays "frozen" with respect to live items — nothing
   re-reads localStorage mid-session, so the double-count invariant holds):

```ts
export function useDesignStore(items: Item[], sessionId: string): DesignStoreApi {
  const [stored, setStored] = useState<StoredBlob>(() => loadBlob(sessionId));
  const [seededFor, setSeededFor] = useState(sessionId);
  const [sent, setSent] = useState<Record<string, Pin[]>>(stored.sent);
  if (seededFor !== sessionId) {
    setSeededFor(sessionId);
    const fresh = loadBlob(sessionId);
    setStored(fresh);
    setSent(fresh.sent);
  }
  const designs = useMemo(() => mergeDesigns(stored.designs, designsFrom(items)), [stored, items]);

  useEffect(() => { saveBlob(sessionId, { designs, sent }); }, [sessionId, designs, sent]);

  return {
    designs,
    sentPins: (id, version) => sent[`${id}@${version}`] ?? [],
    recordSent: (id, version, pins) =>
      setSent((s) => ({ ...s, [`${id}@${version}`]: [...(s[`${id}@${version}`] ?? []), ...pins] })),
    addUrlVersion: (url) => setStored((s) => {
      const version: DesignVersion = {
        display: { Url: { url, id: LIVE_PREVIEW_ID, title: "Live preview" } },
        renderable: true,
      };
      const exists = s.designs.some((d) => d.id === LIVE_PREVIEW_ID);
      const designs = exists
        ? s.designs.map((d) => d.id === LIVE_PREVIEW_ID
            ? cap({ ...d, versions: [...d.versions, version] }) : d)
        : [...s.designs, { id: LIVE_PREVIEW_ID, title: "Live preview", versions: [version] }];
      return { ...s, designs };
    }),
  };
}
```

Note: the doc comment above the hook mentioning "FROZEN at mount" should be updated to say the blob is loaded once per (mount, sessionId) and extended only by `addUrlVersion`. The old `// eslint-disable-next-line react-hooks/exhaustive-deps` above the removed `useMemo` goes away.

In `web/src/components/design/DesignPane.tsx`, add the manual field. New imports:

```tsx
import { useDesignStore, LIVE_PREVIEW_ID } from "../../designStore";
import { isLocalUrl } from "../inspector/urlGuard";
```

Add state next to `activeId`:

```tsx
  const [urlDraft, setUrlDraft] = useState("");
  const [urlError, setUrlError] = useState<string | null>(null);
  const preview = () => {
    if (!isLocalUrl(urlDraft)) {
      setUrlError("Only localhost URLs (e.g. http://localhost:5173) can be previewed.");
      return;
    }
    store.addUrlVersion(urlDraft);
    setActiveId(LIVE_PREVIEW_ID);
    setUrlError(null);
  };
```

Insert the form as the first child of the outer flex column (above the empty state / canvas):

```tsx
      <div className="px-2 pt-2">
        <div className="flex gap-1">
          <input aria-label="preview url" value={urlDraft} placeholder="http://localhost:5173"
            onChange={(e) => setUrlDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") preview(); }}
            className="min-w-0 flex-1 rounded px-2 py-1 text-xs"
            style={{ background: "var(--surface-base)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }} />
          <button onClick={preview} className="rounded px-2 py-1 text-xs"
            style={{ background: "var(--surface-raised)", color: "var(--text-strong)",
              border: "1px solid var(--border)" }}>Preview</button>
        </div>
        {urlError && <p className="pt-1 text-xs" style={{ color: "var(--text-muted)" }}>{urlError}</p>}
      </div>
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run src/designStore.test.ts src/components/design/ && npm run typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/designStore.ts web/src/designStore.test.ts \
  web/src/components/design/DesignPane.tsx web/src/components/design/DesignPane.test.tsx
git commit -m "feat(web): manual localhost url field feeds a live-preview design"
```

---

### Task 9: Full CI gate

**Files:** none (verification only).

- [ ] **Step 1: Run the repo CI script**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: fmt + clippy + `cargo test` (agent workspace) + web typecheck + vitest all green. Fix anything it flags before proceeding (each fix amends or follows the task commit it belongs to).

- [ ] **Step 2: Sanity-check the wire round-trip**

Run (from `agent/`): `source ~/.cargo/env; cargo test -p agent-tools`
Run (from `web/`): `npm test`
Expected: PASS — `Display::Url` serializes as `{"Url":{"url":...}}` (serde external tagging), which is exactly the `wire.ts` shape added in Task 5.

- [ ] **Step 3: Commit anything outstanding and stop for review**

No auto-merge: finish with superpowers:finishing-a-development-branch.
