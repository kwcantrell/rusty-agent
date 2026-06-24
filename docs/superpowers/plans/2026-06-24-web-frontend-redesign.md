# Web Frontend Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure and restyle the `web/` React SPA into a two-pane "builder" layout (left agent column + dominant right Preview/Code workspace) with a light editorial aesthetic, reusing existing components and touching no backend/wire code.

**Architecture:** A new top-level shell (`TopBar` + `AgentColumn` + `WorkspacePane`) replaces today's `StatusBar + (ActivityRail | MessageList | Inspector) + Composer`. The conversation and artifact components are reused as-is and re-composed; the look comes from a centralized design-token + self-hosted-font pass. `socket.ts`, `wire.ts`, `state.ts` are untouched (`storage.ts` gets one additive key for workspace-view persistence).

**Tech Stack:** React 19 + TypeScript, Vite 7, Tailwind CSS v4 (`@theme`/CSS vars), Vitest + jsdom + Testing Library, `@fontsource-variable` (self-hosted fonts). Spec: [`../specs/2026-06-24-web-frontend-redesign-design.md`](../specs/2026-06-24-web-frontend-redesign-design.md).

## Global Constraints

- **Work in `web/`.** Gates: `npm test` (Vitest) and `npm run build` (`tsc -b && vite build`) must both be green after every task. Dev preview: `npm run dev`.
- **No backend/wire/cloud change.** Do NOT modify `src/socket.ts`, `src/wire.ts`, or the logic in `src/state.ts`. `src/storage.ts` may gain ONLY additive functions.
- **Reuse, don't rewrite.** Existing components (`MessageList`, `ToolCall`, `ReasoningMessage`, `DiffView`, `ApprovalPrompt`, `Composer`, `SettingsPanel`, `PairingScreen`, `ArtifactRenderer`, `HtmlArtifact`, `MermaidArtifact`, etc.) keep their logic; only their classes/tokens change.
- **Self-hosted fonts only** — vendored via `@fontsource-variable/*` and imported in `main.tsx`. NO runtime Google-CDN `<link>`/`@import`. The app is served same-origin by the Worker and must work offline.
- **Design tokens live in `src/index.css`** (`:root[data-theme="light"|"dark"]`). Components reference `var(--token)`; no hard-coded hexes in components.
- **Light is primary; dark stays working.** The existing `applyTheme`/`ThemeToggle`/`data-theme` mechanism is unchanged.
- **Display serif is accent/display only** (titles/headings), never body text.
- **Tabs/segmented controls keyboard-accessible** (`role="tab"`/`role="tablist"`, `aria-selected`). `prefers-reduced-motion` already handled globally in `index.css` — keep it.

---

## File Structure

```
web/
  package.json                                  + @fontsource-variable/fraunces, /inter
  src/main.tsx                                  + font imports
  src/index.css                                 editorial tokens + --font-display/--font-sans
  src/storage.ts                                + loadWorkspaceView/saveWorkspaceView (additive)
  src/components/
    TopBar.tsx                  (new)           slim top chrome; replaces StatusBar
    AgentHeader.tsx             (new)           serif project title + model + session
    AgentColumn.tsx             (new)           header + MessageList + ApprovalPrompt + Composer
    workspace/
      WorkspacePane.tsx         (new)           tabs + Preview│Code + viewport + body
      WorkspaceEmptyState.tsx   (new)           editorial empty placeholder
      artifactSource.ts         (new)           Display → {source,lang} | null  (Code-tab text)
    StatusBar.tsx               (delete)        folded into TopBar
    ActivityRail.tsx            (delete)        icon rail removed
    inspector/Inspector.tsx     (delete)        replaced by WorkspacePane (keep ArtifactRenderer/Html/Mermaid)
  src/App.tsx                                   recompose into TopBar + AgentColumn + WorkspacePane
```

Reused unchanged (logic): `MessageList`, `AnimatedAssistantMessage`, `AnimatedToolCall`, `AnimatedReasoningMessage`, `AnimatedError`, `DiffView`, `TerminalBlock`, `MarkdownText`, `ApprovalPrompt`, `Composer`, `SettingsPanel`, `PairingScreen`, `ThemeToggle`, `inspector/ArtifactRenderer`, `inspector/HtmlArtifact`, `inspector/MermaidArtifact`, `socket`, `wire`, `state`.

---

## Task 1: Editorial design tokens + self-hosted fonts

**Files:**
- Modify: `web/package.json` (deps)
- Modify: `web/src/main.tsx`
- Modify: `web/src/index.css`

**Interfaces:**
- Produces: CSS vars `--font-display`, `--font-sans` and a retuned light token palette (ink accent). A `.font-display` utility class.

- [ ] **Step 1: Add the self-hosted font packages**

Run: `cd web && npm install @fontsource-variable/fraunces @fontsource-variable/inter`
Expected: both appear under `dependencies` in `web/package.json`.

- [ ] **Step 2: Import the fonts in `main.tsx`**

At the TOP of `web/src/main.tsx` (before the existing imports), add:
```ts
import "@fontsource-variable/inter";
import "@fontsource-variable/fraunces";
```

- [ ] **Step 3: Add font vars + editorial accent to `index.css`**

In `web/src/index.css`, inside `:root[data-theme="light"]` add these lines (keep the existing surface/text vars; only CHANGE `--accent`/`--accent-fg` and ADD the two font vars):
```css
  --accent: #1c1e1d;
  --accent-fg: #faf9f6;
  --font-sans: "Inter Variable", ui-sans-serif, system-ui, -apple-system, sans-serif;
  --font-display: "Fraunces Variable", ui-serif, Georgia, "Times New Roman", serif;
```
Inside `:root[data-theme="dark"]` CHANGE `--accent`/`--accent-fg` and ADD the same two font vars (dark uses a light ink so primary pills read on the dark base):
```css
  --accent: #e8e6df;
  --accent-fg: #16181a;
  --font-sans: "Inter Variable", ui-sans-serif, system-ui, -apple-system, sans-serif;
  --font-display: "Fraunces Variable", ui-serif, Georgia, "Times New Roman", serif;
```

- [ ] **Step 4: Point the base font at the sans var + add the display utility**

In `web/src/index.css`, change the `body { … font-family: … }` line to:
```css
  font-family: var(--font-sans);
```
And append at the end of the file:
```css
.font-display { font-family: var(--font-display); }
```

- [ ] **Step 5: Verify build + existing tests still pass**

Run: `cd web && npm run build && npm test`
Expected: build succeeds (fonts bundled by Vite); all existing tests pass. (CSS tokens have no unit test; the gate is a clean build.)

- [ ] **Step 6: Commit**

```bash
cd web && git add package.json package-lock.json src/main.tsx src/index.css
git commit -m "feat(web): editorial design tokens + self-hosted Fraunces/Inter fonts"
```

---

## Task 2: `artifactSource` helper (Code-tab text extraction)

**Files:**
- Create: `web/src/components/workspace/artifactSource.ts`
- Create: `web/src/components/workspace/artifactSource.test.ts`

**Interfaces:**
- Consumes: `Display` from `../../wire`.
- Produces: `artifactSource(d: Display): { source: string; lang: string } | null` — returns the raw source + a highlight language for renderable artifacts, or `null` for ones with no meaningful source (Code tab disabled).

- [ ] **Step 1: Write the failing test**

`web/src/components/workspace/artifactSource.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { artifactSource } from "./artifactSource";
import type { Display } from "../../wire";

describe("artifactSource", () => {
  it("extracts html source", () => {
    const d = { Html: { html: "<h1>hi</h1>" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "<h1>hi</h1>", lang: "html" });
  });
  it("extracts mermaid source", () => {
    const d = { Mermaid: { source: "graph TD; A-->B" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "graph TD; A-->B", lang: "mermaid" });
  });
  it("extracts code source with its language", () => {
    const d = { Code: { filename: "a.rs", lang: "rust", text: "fn main() {}" } } as unknown as Display;
    expect(artifactSource(d)).toEqual({ source: "fn main() {}", lang: "rust" });
  });
  it("extracts plain text and markdown", () => {
    expect(artifactSource({ Text: "hello" } as unknown as Display)).toEqual({ source: "hello", lang: "text" });
    expect(artifactSource({ Markdown: { text: "# h" } } as unknown as Display)).toEqual({ source: "# h", lang: "markdown" });
  });
  it("returns null for non-source displays", () => {
    expect(artifactSource({ Diff: { path: "a", before: "x", after: "y" } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Terminal: { command: "ls", stdout: "", stderr: "", exit_code: 0 } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Image: { mime: "image/png", data: "..." } } as unknown as Display)).toBeNull();
    expect(artifactSource({ Table: { columns: [], rows: [] } } as unknown as Display)).toBeNull();
  });
});
```

- [ ] **Step 2: Run the test to confirm it fails**

Run: `cd web && npx vitest run src/components/workspace/artifactSource.test.ts`
Expected: FAIL — cannot resolve `./artifactSource`.

- [ ] **Step 3: Implement the helper**

`web/src/components/workspace/artifactSource.ts`:
```ts
import type { Display } from "../../wire";

/** The raw source + highlight language for the Code tab, or null when an
 *  artifact has no meaningful source (Diff/Terminal/Table/Image → Code disabled). */
export function artifactSource(d: Display): { source: string; lang: string } | null {
  if ("Html" in d) return { source: d.Html.html, lang: "html" };
  if ("Mermaid" in d) return { source: d.Mermaid.source, lang: "mermaid" };
  if ("Code" in d) return { source: d.Code.text, lang: d.Code.lang };
  if ("Text" in d) return { source: d.Text, lang: "text" };
  if ("Markdown" in d) return { source: d.Markdown.text, lang: "markdown" };
  return null;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cd web && npx vitest run src/components/workspace/artifactSource.test.ts`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
cd web && git add src/components/workspace/artifactSource.ts src/components/workspace/artifactSource.test.ts
git commit -m "feat(web): artifactSource helper for the workspace Code tab"
```

---

## Task 3: Workspace-view persistence (additive storage)

**Files:**
- Modify: `web/src/storage.ts`
- Create: `web/src/storage.workspace.test.ts`

**Interfaces:**
- Produces: `WorkspaceView = { mode: "preview" | "code"; viewport: "desktop" | "tablet" | "mobile" }`; `loadWorkspaceView(): WorkspaceView` (defaults `{mode:"preview",viewport:"desktop"}`); `saveWorkspaceView(v: WorkspaceView): void`.

- [ ] **Step 1: Write the failing test**

`web/src/storage.workspace.test.ts`:
```ts
import { describe, it, expect, beforeEach } from "vitest";
import { loadWorkspaceView, saveWorkspaceView } from "./storage";

describe("workspace view persistence", () => {
  beforeEach(() => localStorage.clear());
  it("defaults to preview/desktop", () => {
    expect(loadWorkspaceView()).toEqual({ mode: "preview", viewport: "desktop" });
  });
  it("round-trips a saved view", () => {
    saveWorkspaceView({ mode: "code", viewport: "mobile" });
    expect(loadWorkspaceView()).toEqual({ mode: "code", viewport: "mobile" });
  });
  it("falls back to defaults on garbage", () => {
    localStorage.setItem("agent.workspaceView", "not json");
    expect(loadWorkspaceView()).toEqual({ mode: "preview", viewport: "desktop" });
  });
});
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cd web && npx vitest run src/storage.workspace.test.ts`
Expected: FAIL — `loadWorkspaceView` is not exported.

- [ ] **Step 3: Add the additive functions**

Append to `web/src/storage.ts`:
```ts
export type WorkspaceView = {
  mode: "preview" | "code";
  viewport: "desktop" | "tablet" | "mobile";
};
const WORKSPACE_VIEW = "agent.workspaceView";
const DEFAULT_VIEW: WorkspaceView = { mode: "preview", viewport: "desktop" };

export function loadWorkspaceView(): WorkspaceView {
  const raw = localStorage.getItem(WORKSPACE_VIEW);
  if (!raw) return { ...DEFAULT_VIEW };
  try {
    const v = JSON.parse(raw) as Partial<WorkspaceView>;
    const mode = v.mode === "code" ? "code" : "preview";
    const viewport = v.viewport === "tablet" || v.viewport === "mobile" ? v.viewport : "desktop";
    return { mode, viewport };
  } catch {
    return { ...DEFAULT_VIEW };
  }
}
export function saveWorkspaceView(v: WorkspaceView): void {
  try { localStorage.setItem(WORKSPACE_VIEW, JSON.stringify(v)); } catch { /* ignore */ }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd web && npx vitest run src/storage.workspace.test.ts`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd web && git add src/storage.ts src/storage.workspace.test.ts
git commit -m "feat(web): persist workspace mode/viewport (additive storage)"
```

---

## Task 4: WorkspacePane (tabs + Preview/Code + viewport + empty state)

**Files:**
- Create: `web/src/components/workspace/WorkspaceEmptyState.tsx`
- Create: `web/src/components/workspace/WorkspacePane.tsx`
- Create: `web/src/components/workspace/WorkspacePane.test.tsx`

**Interfaces:**
- Consumes: `InspectorArtifact` from `../../state`; `ArtifactRenderer` from `../inspector/ArtifactRenderer`; `MarkdownText` from `../MarkdownText`; `artifactSource` (Task 2); `WorkspaceView`/`loadWorkspaceView`/`saveWorkspaceView` (Task 3).
- Produces: `WorkspacePane({ artifacts, activeKey, onSelect }: { artifacts: InspectorArtifact[]; activeKey: string | null; onSelect: (key: string) => void })`.

- [ ] **Step 1: Write the empty-state component**

`web/src/components/workspace/WorkspaceEmptyState.tsx`:
```tsx
export function WorkspaceEmptyState() {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center"
      style={{ color: "var(--text-muted)" }}>
      <div className="font-display text-2xl" style={{ color: "var(--text-strong)" }}>Workspace</div>
      <div className="text-sm">Rendered output from the agent will appear here.</div>
    </div>
  );
}
```

- [ ] **Step 2: Write the failing test**

`web/src/components/workspace/WorkspacePane.test.tsx`:
```tsx
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent, within } from "@testing-library/react";
import { WorkspacePane } from "./WorkspacePane";
import type { InspectorArtifact } from "../../state";

const htmlArt: InspectorArtifact = { key: "art-1", title: "page.html", display: { Html: { html: "<h1>Hi</h1>" } } as never };
const tableArt: InspectorArtifact = { key: "art-2", title: "data", display: { Table: { columns: ["a"], rows: [["1"]] } } as never };

describe("WorkspacePane", () => {
  beforeEach(() => localStorage.clear());

  it("shows the empty state with no artifacts", () => {
    render(<WorkspacePane artifacts={[]} activeKey={null} onSelect={() => {}} />);
    expect(screen.getByText("Workspace")).toBeInTheDocument();
  });

  it("renders a tab per artifact and selects on click", () => {
    const selected: string[] = [];
    render(<WorkspacePane artifacts={[htmlArt, tableArt]} activeKey="art-1" onSelect={(k) => selected.push(k)} />);
    expect(screen.getByRole("tab", { name: /page.html/ })).toHaveAttribute("aria-selected", "true");
    fireEvent.click(screen.getByRole("tab", { name: /data/ }));
    expect(selected).toContain("art-2");
  });

  it("toggles Preview/Code; Code shows source", () => {
    render(<WorkspacePane artifacts={[htmlArt]} activeKey="art-1" onSelect={() => {}} />);
    // Preview by default → the sandboxed iframe is present
    expect(screen.getByTitle("rendered html")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /^Code$/ }));
    // Code view shows the raw source text
    expect(screen.getByText(/<h1>Hi<\/h1>/)).toBeInTheDocument();
  });

  it("disables Code when the active artifact has no source", () => {
    render(<WorkspacePane artifacts={[tableArt]} activeKey="art-2" onSelect={() => {}} />);
    expect(screen.getByRole("button", { name: /^Code$/ })).toBeDisabled();
  });

  it("viewport selector constrains the preview width and is disabled in Code mode", () => {
    render(<WorkspacePane artifacts={[htmlArt]} activeKey="art-1" onSelect={() => {}} />);
    const frame = screen.getByTestId("preview-frame");
    expect(frame).toHaveStyle({ maxWidth: "100%" }); // desktop default
    fireEvent.click(screen.getByRole("button", { name: /Mobile/ }));
    expect(screen.getByTestId("preview-frame")).toHaveStyle({ maxWidth: "390px" });
    // switch to Code → viewport buttons disabled
    fireEvent.click(screen.getByRole("button", { name: /^Code$/ }));
    expect(screen.getByRole("button", { name: /Desktop/ })).toBeDisabled();
  });
});
```

- [ ] **Step 3: Run to confirm it fails**

Run: `cd web && npx vitest run src/components/workspace/WorkspacePane.test.tsx`
Expected: FAIL — cannot resolve `./WorkspacePane`.

- [ ] **Step 4: Implement `WorkspacePane`**

`web/src/components/workspace/WorkspacePane.tsx`:
```tsx
import { useState } from "react";
import type { InspectorArtifact } from "../../state";
import { ArtifactRenderer } from "../inspector/ArtifactRenderer";
import { MarkdownText } from "../MarkdownText";
import { artifactSource } from "./artifactSource";
import { loadWorkspaceView, saveWorkspaceView, type WorkspaceView } from "../../storage";
import { WorkspaceEmptyState } from "./WorkspaceEmptyState";

const VIEWPORTS: { id: WorkspaceView["viewport"]; label: string; maxWidth: string }[] = [
  { id: "desktop", label: "Desktop", maxWidth: "100%" },
  { id: "tablet", label: "Tablet", maxWidth: "820px" },
  { id: "mobile", label: "Mobile", maxWidth: "390px" },
];

export function WorkspacePane({ artifacts, activeKey, onSelect }:
  { artifacts: InspectorArtifact[]; activeKey: string | null; onSelect: (key: string) => void }) {
  const [view, setView] = useState<WorkspaceView>(() => loadWorkspaceView());
  const setMode = (mode: WorkspaceView["mode"]) => { const v = { ...view, mode }; setView(v); saveWorkspaceView(v); };
  const setViewport = (viewport: WorkspaceView["viewport"]) => { const v = { ...view, viewport }; setView(v); saveWorkspaceView(v); };

  if (artifacts.length === 0) {
    return (
      <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
        <WorkspaceEmptyState />
      </div>
    );
  }

  const active = artifacts.find((a) => a.key === activeKey) ?? artifacts[artifacts.length - 1];
  const source = artifactSource(active.display);
  const codeDisabled = source === null;
  // If Code is selected but this artifact has no source, fall back to Preview for rendering.
  const mode = view.mode === "code" && !codeDisabled ? "code" : "preview";
  const vp = VIEWPORTS.find((v) => v.id === view.viewport) ?? VIEWPORTS[0];

  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-overlay)" }}>
      {/* header: tabs */}
      <div className="flex items-center gap-1 overflow-x-auto px-2 pt-2" role="tablist"
        style={{ borderBottom: "1px solid var(--border)" }}>
        {artifacts.map((a) => {
          const on = a.key === active.key;
          return (
            <button key={a.key} role="tab" aria-selected={on} onClick={() => onSelect(a.key)}
              className="whitespace-nowrap rounded-t-lg px-3 py-1.5 text-xs"
              style={{
                background: on ? "var(--surface-overlay)" : "transparent",
                color: on ? "var(--text-strong)" : "var(--text-muted)",
                fontWeight: on ? 600 : 400,
                border: on ? "1px solid var(--border)" : "1px solid transparent",
                borderBottom: "none",
              }}>
              {a.title}
            </button>
          );
        })}
      </div>
      {/* header: mode toggle + viewport */}
      <div className="flex items-center justify-between gap-2 px-3 py-2"
        style={{ borderBottom: "1px solid var(--border)" }}>
        <div className="inline-flex rounded-full p-0.5" style={{ border: "1px solid var(--border)" }}>
          {(["preview", "code"] as const).map((m) => {
            const on = mode === m;
            const disabled = m === "code" && codeDisabled;
            return (
              <button key={m} onClick={() => setMode(m)} disabled={disabled}
                title={disabled ? "This artifact has no source to show" : undefined}
                className="rounded-full px-3 py-1 text-xs capitalize disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ background: on ? "var(--accent)" : "transparent", color: on ? "var(--accent-fg)" : "var(--text-muted)" }}>
                {m}
              </button>
            );
          })}
        </div>
        <div className="inline-flex gap-1">
          {VIEWPORTS.map((v) => {
            const on = view.viewport === v.id;
            return (
              <button key={v.id} onClick={() => setViewport(v.id)} disabled={mode === "code"}
                className="rounded-full px-2.5 py-1 text-xs disabled:opacity-40 disabled:cursor-not-allowed"
                style={{ background: on ? "var(--surface-raised)" : "transparent",
                         color: on ? "var(--text-strong)" : "var(--text-muted)",
                         border: "1px solid " + (on ? "var(--border)" : "transparent") }}>
                {v.label}
              </button>
            );
          })}
        </div>
      </div>
      {/* body */}
      <div className="min-h-0 flex-1 overflow-auto p-3">
        {mode === "preview" ? (
          <div data-testid="preview-frame" className="mx-auto h-full"
            style={{ maxWidth: vp.maxWidth, width: "100%" }}>
            <ArtifactRenderer display={active.display} />
          </div>
        ) : (
          <MarkdownText text={"```" + (source?.lang ?? "text") + "\n" + (source?.source ?? "") + "\n```"} />
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd web && npx vitest run src/components/workspace/WorkspacePane.test.tsx`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
cd web && git add src/components/workspace/
git commit -m "feat(web): WorkspacePane — artifact tabs, Preview/Code, viewport selector, empty state"
```

---

## Task 5: TopBar (replaces StatusBar)

**Files:**
- Create: `web/src/components/TopBar.tsx`
- Create: `web/src/components/TopBar.test.tsx`
- Delete: `web/src/components/StatusBar.tsx` (in Task 7, once App stops importing it)

**Interfaces:**
- Consumes: `ConnectionStatus` from `../state`; `Theme` from `../theme`; `ThemeToggle` from `./ThemeToggle`.
- Produces: `TopBar({ projectLabel, online, status, theme, onToggleTheme, onOpenSettings, settingsDisabled, onSignOut, onToggleWorkspace, showWorkspaceToggle }: { projectLabel: string; online: boolean; status: ConnectionStatus; theme: Theme; onToggleTheme: () => void; onOpenSettings?: () => void; settingsDisabled?: boolean; onSignOut: () => void; onToggleWorkspace?: () => void; showWorkspaceToggle?: boolean })`.

- [ ] **Step 1: Write the failing test**

`web/src/components/TopBar.test.tsx`:
```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TopBar } from "./TopBar";

const base = { projectLabel: "studio-x", online: true, status: "open" as const,
  theme: "light" as const, onToggleTheme: () => {}, onSignOut: () => {} };

describe("TopBar", () => {
  it("shows the project label and online state", () => {
    render(<TopBar {...base} />);
    expect(screen.getByText("studio-x")).toBeInTheDocument();
  });
  it("opens settings and signs out", () => {
    const onOpenSettings = vi.fn(); const onSignOut = vi.fn();
    render(<TopBar {...base} onOpenSettings={onOpenSettings} onSignOut={onSignOut} />);
    fireEvent.click(screen.getByLabelText("settings"));
    fireEvent.click(screen.getByRole("button", { name: /sign out/i }));
    expect(onOpenSettings).toHaveBeenCalled();
    expect(onSignOut).toHaveBeenCalled();
  });
  it("shows the workspace toggle only when asked", () => {
    const onToggleWorkspace = vi.fn();
    const { rerender } = render(<TopBar {...base} />);
    expect(screen.queryByLabelText("toggle workspace")).toBeNull();
    rerender(<TopBar {...base} showWorkspaceToggle onToggleWorkspace={onToggleWorkspace} />);
    fireEvent.click(screen.getByLabelText("toggle workspace"));
    expect(onToggleWorkspace).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run to confirm it fails**

Run: `cd web && npx vitest run src/components/TopBar.test.tsx`
Expected: FAIL — cannot resolve `./TopBar`.

- [ ] **Step 3: Implement `TopBar`**

`web/src/components/TopBar.tsx`:
```tsx
import type { ConnectionStatus } from "../state";
import type { Theme } from "../theme";
import { ThemeToggle } from "./ThemeToggle";

export function TopBar({ projectLabel, online, status, theme, onToggleTheme,
  onOpenSettings, settingsDisabled, onSignOut, onToggleWorkspace, showWorkspaceToggle }:
  { projectLabel: string; online: boolean; status: ConnectionStatus;
    theme: Theme; onToggleTheme: () => void;
    onOpenSettings?: () => void; settingsDisabled?: boolean; onSignOut: () => void;
    onToggleWorkspace?: () => void; showWorkspaceToggle?: boolean }) {
  return (
    <div className="flex items-center justify-between px-4 py-2.5"
      style={{ background: "var(--surface-base)", borderBottom: "1px solid var(--border)" }}>
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full"
          style={{ background: online ? "var(--state-done)" : "var(--text-muted)" }}
          title={online ? "agent online" : "agent offline"} />
        <span className="font-display text-base" style={{ color: "var(--text-strong)" }}>{projectLabel}</span>
        <span className="text-xs" style={{ color: "var(--text-muted)" }}>· {status}</span>
      </div>
      <div className="flex items-center gap-3 text-sm">
        {showWorkspaceToggle && (
          <button onClick={onToggleWorkspace} aria-label="toggle workspace"
            className="rounded-full px-3 py-1 text-xs hover:opacity-80"
            style={{ border: "1px solid var(--border)", color: "var(--text)" }}>Workspace</button>
        )}
        <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        {onOpenSettings && (
          <button onClick={onOpenSettings} disabled={settingsDisabled} aria-label="settings"
            className="disabled:opacity-40 disabled:cursor-not-allowed hover:opacity-80"
            style={{ color: "var(--text-muted)" }}>⚙</button>
        )}
        <button onClick={onSignOut} className="hover:opacity-80" style={{ color: "var(--text-muted)" }}>sign out</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd web && npx vitest run src/components/TopBar.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
cd web && git add src/components/TopBar.tsx src/components/TopBar.test.tsx
git commit -m "feat(web): TopBar chrome (project label + controls; supersedes StatusBar)"
```

---

## Task 6: AgentHeader + AgentColumn

**Files:**
- Create: `web/src/components/AgentHeader.tsx`
- Create: `web/src/components/AgentColumn.tsx`
- Create: `web/src/components/AgentColumn.test.tsx`

**Interfaces:**
- Consumes: `AnimatedItem`, `PendingApproval` from `../state`; `Decision` from `../wire`; `MessageList`, `ApprovalPrompt`, `Composer`.
- Produces:
  - `AgentHeader({ projectLabel, model }: { projectLabel: string; model?: string })`.
  - `AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model, pendingApproval, onDecide, composerDisabled, onSend }: { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void; projectLabel: string; model?: string; pendingApproval: PendingApproval | null; onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void })`.

- [ ] **Step 1: Write `AgentHeader`**

`web/src/components/AgentHeader.tsx`:
```tsx
export function AgentHeader({ projectLabel, model }: { projectLabel: string; model?: string }) {
  return (
    <div className="px-4 pb-3 pt-4" style={{ borderBottom: "1px solid var(--border)" }}>
      <div className="font-display text-xl leading-tight" style={{ color: "var(--text-strong)" }}>{projectLabel}</div>
      {model && (
        <div className="mt-0.5 font-mono text-xs" style={{ color: "var(--text-muted)" }}>model {model}</div>
      )}
    </div>
  );
}
```

- [ ] **Step 2: Write the failing test for `AgentColumn`**

`web/src/components/AgentColumn.test.tsx`:
```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AgentColumn } from "./AgentColumn";

const base = {
  items: [], activeArtifactKey: null, onSelectArtifact: () => {},
  projectLabel: "studio-x", model: "qwen3", pendingApproval: null,
  onDecide: () => {}, composerDisabled: false, onSend: vi.fn(),
};

describe("AgentColumn", () => {
  it("renders the header (project + model) and an enabled composer", () => {
    render(<AgentColumn {...base} />);
    expect(screen.getByText("studio-x")).toBeInTheDocument();
    expect(screen.getByText(/model qwen3/)).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Message the agent/)).toBeEnabled();
  });
  it("disables the composer when asked", () => {
    render(<AgentColumn {...base} composerDisabled />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });
  it("sends a message", () => {
    const onSend = vi.fn();
    render(<AgentColumn {...base} onSend={onSend} />);
    const ta = screen.getByPlaceholderText(/Message the agent/);
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello");
  });
});
```

- [ ] **Step 3: Run to confirm it fails**

Run: `cd web && npx vitest run src/components/AgentColumn.test.tsx`
Expected: FAIL — cannot resolve `./AgentColumn`.

- [ ] **Step 4: Implement `AgentColumn`**

`web/src/components/AgentColumn.tsx`:
```tsx
import type { AnimatedItem, PendingApproval } from "../state";
import type { Decision } from "../wire";
import { AgentHeader } from "./AgentHeader";
import { MessageList } from "./MessageList";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { Composer } from "./Composer";

export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void }) {
  return (
    <div className="flex h-full min-h-0 flex-col" style={{ background: "var(--surface-base)" }}>
      <AgentHeader projectLabel={projectLabel} model={model} />
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
      </div>
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <Composer disabled={composerDisabled} onSend={onSend} />
    </div>
  );
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd web && npx vitest run src/components/AgentColumn.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
cd web && git add src/components/AgentHeader.tsx src/components/AgentColumn.tsx src/components/AgentColumn.test.tsx
git commit -m "feat(web): AgentColumn (serif header + conversation + approval + composer)"
```

---

## Task 7: Recompose `App.tsx` into the two-pane shell + responsive

**Files:**
- Modify: `web/src/App.tsx`
- Delete: `web/src/components/StatusBar.tsx`, `web/src/components/ActivityRail.tsx`, `web/src/components/inspector/Inspector.tsx`
- Create: `web/src/App.test.tsx`

**Interfaces:**
- Consumes: `TopBar` (Task 5), `AgentColumn` (Task 6), `WorkspacePane` (Task 4). Reuses existing `App` state/handlers (`send`, `decide`, `signOut`, `openSettings`, `saveSettings`, `activeArtifactKey`, `artifactsFrom`, `useAnimatedItems`).

- [ ] **Step 1: Confirm nothing else imports the components being deleted**

Run: `cd web && grep -rln "ActivityRail\|StatusBar\|inspector/Inspector\|from \"./components/Inspector\"" src | grep -v App.tsx`
Expected: NO output (only `App.tsx` references them). If anything else shows up, stop and report it.

- [ ] **Step 2: Write the App smoke test (unauthenticated path — no socket needed)**

`web/src/App.test.tsx`:
```tsx
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import App from "./App";

describe("App shell", () => {
  beforeEach(() => localStorage.clear());
  it("renders the pairing screen when unauthenticated", () => {
    render(<App />);
    // PairingScreen shows a pairing-code affordance; assert it is on screen, not the two-pane shell.
    expect(screen.queryByText(/sign out/i)).toBeNull();
  });
});
```

- [ ] **Step 3: Run to confirm it passes against the CURRENT App (baseline)**

Run: `cd web && npx vitest run src/App.test.tsx`
Expected: PASS (the current App already renders `PairingScreen` when unauthenticated). This pins behavior we must preserve through the rewrite.

- [ ] **Step 4: Rewrite `App.tsx`'s render to the two-pane shell**

Replace the imports of `StatusBar`, `ActivityRail`, `Inspector` with `TopBar`, `AgentColumn`, `WorkspacePane`, and replace the authenticated `return (...)` block. The full new `App.tsx` body from the `connected`/`return` onward:

```tsx
  const connected = state.status === "open";
  const projectLabel = `session ${sessionId.slice(0, 8)}`;
  const model = state.settings?.model;
  const narrow = useNarrow();

  return (
    <div className="flex h-screen flex-col" style={{ background: "var(--surface-base)" }}>
      <TopBar projectLabel={projectLabel} online={state.online} status={state.status}
        theme={theme} onToggleTheme={toggleTheme}
        onOpenSettings={openSettings} settingsDisabled={!(connected && state.online)}
        onSignOut={signOut}
        showWorkspaceToggle={narrow} onToggleWorkspace={() => setWorkspaceOpen((o) => !o)} />
      {showSettings && state.settings && (
        <SettingsPanel settings={state.settings} meta={state.settingsMeta} error={state.settingsError}
          disabled={!connected} onSave={saveSettings} onClose={() => setShowSettings(false)} />
      )}
      <div className="flex min-h-0 flex-1">
        <div className="min-w-0 flex-1" style={!narrow ? { flexBasis: "38%", maxWidth: "42%", borderRight: "1px solid var(--border)" } : undefined}>
          <AgentColumn items={animatedItems} activeArtifactKey={activeArtifactKey}
            onSelectArtifact={(key) => { setActiveArtifactKey(key); setWorkspaceOpen(true); }}
            projectLabel={projectLabel} model={model}
            pendingApproval={state.pendingApproval} onDecide={decide}
            composerDisabled={!connected} onSend={send} />
        </div>
        {!narrow && (
          <div className="min-w-0 flex-1">
            <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
          </div>
        )}
        {narrow && workspaceOpen && (
          <div className="absolute inset-0 z-20" style={{ background: "var(--surface-overlay)" }}>
            <div className="flex items-center justify-end p-2" style={{ borderBottom: "1px solid var(--border)" }}>
              <button onClick={() => setWorkspaceOpen(false)} aria-label="close workspace"
                className="px-2 text-sm" style={{ color: "var(--text-muted)" }}>✕</button>
            </div>
            <div className="h-[calc(100%-2.5rem)]">
              <WorkspacePane artifacts={artifacts} activeKey={activeArtifactKey} onSelect={setActiveArtifactKey} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
```

Then add the supporting pieces:
1. Replace the three component imports at the top:
```tsx
import { TopBar } from "./components/TopBar";
import { AgentColumn } from "./components/AgentColumn";
import { WorkspacePane } from "./components/workspace/WorkspacePane";
```
   and DELETE the `StatusBar`, `ActivityRail`, `Inspector` import lines.
2. Remove the now-unused `railCollapsed`/`inspectorOpen` state and the `inspectorOpen` effect; ADD:
```tsx
const [workspaceOpen, setWorkspaceOpen] = useState(false);
```
   Keep the existing `activeArtifactKey` state and the effect that auto-selects the latest artifact (drop its `setInspectorOpen(true)` call).
3. Add this hook at the bottom of the file (after `App`):
```tsx
function useNarrow(): boolean {
  const [narrow, setNarrow] = useState(() => window.matchMedia?.("(max-width: 900px)").matches ?? false);
  useEffect(() => {
    const mq = window.matchMedia?.("(max-width: 900px)");
    if (!mq) return;
    const on = () => setNarrow(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return narrow;
}
```
   (`useState`/`useEffect` are already imported.)

- [ ] **Step 5: Delete the retired components**

```bash
cd web && git rm src/components/StatusBar.tsx src/components/ActivityRail.tsx src/components/inspector/Inspector.tsx
```

- [ ] **Step 6: Run the full gate**

Run: `cd web && npm run build && npm test`
Expected: `tsc` clean (no unused imports / no references to deleted files), build succeeds, ALL tests pass (App smoke test + every prior test).

- [ ] **Step 7: Commit**

```bash
cd web && git add -A
git commit -m "feat(web): recompose App into TopBar + AgentColumn + WorkspacePane (two-pane, responsive)"
```

---

## Task 8: Editorial polish pass on reused controls + final validation

**Files:**
- Modify: `web/src/components/Composer.tsx`
- Modify: `web/src/components/MessageList.tsx` (user-bubble radius only)
- Modify: `web/src/components/PairingScreen.tsx` (surface/serif heading only)

**Interfaces:** none new — class/style-only edits, no prop or logic changes.

- [ ] **Step 1: Soften the Composer to an editorial input block**

In `web/src/components/Composer.tsx`, change the outer wrapper and the textarea/button classes to use a rounded, pill-button treatment (no behavior change):
- Outer `div` className → `flex gap-2 p-3` with `style={{ background: "var(--surface-base)", borderTop: "1px solid var(--border)" }}` (unchanged).
- textarea className → `flex-1 resize-none rounded-xl p-3 outline-none disabled:opacity-50` (was `rounded-lg p-2`).
- Send button className → `rounded-full px-5 disabled:opacity-50 hover:opacity-90` (was `rounded-lg px-4`).

- [ ] **Step 2: Round the user message bubble**

In `web/src/components/MessageList.tsx`, the `case "user":` bubble className `rounded-lg` → `rounded-2xl` (style/colors unchanged).

- [ ] **Step 3: Give PairingScreen the editorial heading**

In `web/src/components/PairingScreen.tsx`, add `font-display` to its primary heading element and ensure its container uses `var(--surface-base)` (do not change the pairing logic or inputs). If the heading has no class hook, wrap its title text in `<span className="font-display">…</span>`.

- [ ] **Step 4: Run the full gate**

Run: `cd web && npm run build && npm test`
Expected: build + all tests green (these are visual class changes; existing tests that assert text/roles still pass).

- [ ] **Step 5: Validate in the running app (DoD)**

Per `cloud/RUNNING.md`, bring up the full stack and open the SPA. Confirm by eye: two-pane layout (agent left ~38%, workspace right), serif project header, light editorial chrome; trigger the agent to produce an artifact (e.g. an HTML or mermaid tool result) and confirm it appears as a workspace tab, the Preview renders it, the Code tab shows its source, and the viewport selector (Desktop/Tablet/Mobile) resizes the Preview. Toggle the theme (light↔dark). Note: if the stack can't be brought up in this environment, record that and rely on the component + build gates.

- [ ] **Step 6: Commit**

```bash
cd web && git add -A
git commit -m "feat(web): editorial polish on composer, message bubble, pairing screen"
```

---

## Self-Review

**Spec coverage:**
- §2 layout / component mapping → Tasks 5 (TopBar), 6 (AgentColumn/Header), 4 (WorkspacePane), 7 (App recompose + deletions). ✓
- §3 workspace behavior (tabs, Preview/Code, viewport, empty, persistence) → Tasks 2, 3, 4. ✓
- §4 aesthetic (tokens, self-hosted serif+sans, ink accent, pills) → Tasks 1, 8. ✓
- §5 responsiveness (slide-over < 900px) → Task 7 (`useNarrow`, `workspaceOpen`, TopBar toggle). ✓
- §6 a11y (tab roles/aria-selected, focus, reduced-motion preserved) → Task 4 (tablist/tabs), Task 1 (keeps reduced-motion block). ✓
- §7 failure modes (disconnected, empty, code-no-source, settings, pairing) → Tasks 4 (empty + code-disabled), 6/7 (composer disabled, settings, pairing). ✓
- §8 footprint (self-hosted subset fonts, no new heavy deps) → Task 1 (`@fontsource-variable` only). ✓
- §10 testing → tasks ship Vitest tests; Task 7/8 run `npm run build && npm test`. ✓
- §11 DoD → Task 8 Step 5 in-app validation. ✓

**Placeholder scan:** no TBD/TODO; every code step shows the code. The one non-unit-tested deliverable (Task 1 CSS tokens) is explicitly gated on `npm run build` + existing tests, not a vague "verify it looks right."

**Type consistency:** `WorkspaceView`/`loadWorkspaceView`/`saveWorkspaceView` (Task 3) match their use in `WorkspacePane` (Task 4); `artifactSource` return `{source,lang}|null` (Task 2) matches Task 4's `source?.lang`/`source?.source` + `codeDisabled = source === null`; `TopBar`/`AgentColumn`/`WorkspacePane` prop shapes (Tasks 5/6/4) match the call sites in `App.tsx` (Task 7); `InspectorArtifact { key, title, display }` and `artifactsFrom` are used as defined in `state.ts`.
