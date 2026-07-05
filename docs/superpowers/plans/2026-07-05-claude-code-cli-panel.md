# Claude Code–style CLI Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restyle the left agent panel (`web/src/components/AgentColumn.tsx` and children) into a Claude Code–style monospace terminal transcript with inline tool results, a `>` prompt box with history, a status line, and a keyboard-driven approval prompt.

**Architecture:** Approach A from the approved spec (`docs/superpowers/specs/2026-07-05-claude-code-cli-panel-design.md`): restyle the existing component tree in place. New `--cli-*` design tokens scoped to a `cli` class on the panel root; pure formatting helpers in a new `cliFormat.ts`; everything renders from existing `AnimatedItem` fields and existing reducer state (`state.inTurn` for the busy line). No Rust, wire-protocol, or right-pane changes.

**Tech Stack:** React 19, TypeScript, Tailwind 4 (via `@import "tailwindcss"`), framer-motion, vitest + @testing-library/react (jsdom), @fontsource-variable fonts.

## Global Constraints

- **`web/` only.** No changes under `agent/`, `src-tauri/` (except nothing), or the wire protocol.
- All commands run from `/home/kalen/rust-agent-runtime/web` unless stated otherwise.
- Token values are fixed by the spec — copy them **verbatim** (dark/light): `--cli-bg` `#1a1915`/`#faf9f5`, `--cli-text` `#e8e6dc`/`#3d3d38`, `--cli-dim` `#8c8a7d`/`#8a877c`, `--cli-accent` `#d97757`/`#c15f3c`, `--cli-ok` `#6fae72`/`#4f7a52`, `--cli-err` `#e0654f`/`#b3402e`, `--cli-border` `#35332c`/`#e3e0d5`.
- Truncation limits: arg summary 60 chars, result summary 80 chars, expanded preview 20 lines, block meter 10 cells, red at ≥80%.
- No "esc to interrupt" hint anywhere (the runtime cannot interrupt).
- No Send button — Enter is the only send path.
- Entrance animations are opacity-fade only (no `y` slide). Keep `exit={{ opacity: 0 }}`-style exits.
- Conventional commits: `feat(web): …` / `test(web): …` / `refactor(web): …`.
- Every task ends green: `npx vitest run` and `npm run typecheck` must pass before its commit.

---

### Task 1: Design tokens + monospace font foundation

**Files:**
- Modify: `web/package.json` (via npm install)
- Modify: `web/src/main.tsx:1-2`
- Modify: `web/src/index.css`

**Interfaces:**
- Consumes: nothing.
- Produces: CSS variables `--cli-bg`, `--cli-text`, `--cli-dim`, `--cli-accent`, `--cli-ok`, `--cli-err`, `--cli-border`, `--font-cli` in both theme blocks; a `.cli` scoping class and a `.cli-promptbox:focus-within` rule. Every later task's inline styles reference these names exactly.

- [ ] **Step 1: Install the mono font**

Run: `cd /home/kalen/rust-agent-runtime/web && npm install @fontsource-variable/jetbrains-mono`
Expected: `package.json` gains `"@fontsource-variable/jetbrains-mono"` in dependencies.

- [ ] **Step 2: Import it in `src/main.tsx`**

Add after the existing font imports (lines 1–2):

```tsx
import "@fontsource-variable/jetbrains-mono";
```

- [ ] **Step 3: Add tokens and scoping class to `src/index.css`**

Inside `:root[data-theme="light"]` (after the existing `--font-display` line) add:

```css
  --cli-bg: #faf9f5;
  --cli-text: #3d3d38;
  --cli-dim: #8a877c;
  --cli-accent: #c15f3c;
  --cli-ok: #4f7a52;
  --cli-err: #b3402e;
  --cli-border: #e3e0d5;
  --font-cli: "JetBrains Mono Variable", ui-monospace, "SF Mono", monospace;
```

Inside `:root[data-theme="dark"]` (same position) add:

```css
  --cli-bg: #1a1915;
  --cli-text: #e8e6dc;
  --cli-dim: #8c8a7d;
  --cli-accent: #d97757;
  --cli-ok: #6fae72;
  --cli-err: #e0654f;
  --cli-border: #35332c;
  --font-cli: "JetBrains Mono Variable", ui-monospace, "SF Mono", monospace;
```

At the end of the file (after the `.font-display` rule) add:

```css
/* Claude Code-style terminal panel: everything inside .cli is mono, 13px. */
.cli {
  font-family: var(--font-cli);
  font-size: 13px;
  line-height: 1.65;
  background: var(--cli-bg);
  color: var(--cli-text);
}
.cli-promptbox:focus-within { border-color: var(--cli-accent) !important; }
```

- [ ] **Step 4: Verify green**

Run: `npx vitest run && npm run typecheck`
Expected: all existing tests PASS; typecheck clean.

- [ ] **Step 5: Commit**

```bash
git add package.json package-lock.json src/main.tsx src/index.css
git commit -m "feat(web): add cli terminal design tokens and JetBrains Mono"
```

---

### Task 2: Pure formatting helpers (`cliFormat.ts`)

**Files:**
- Create: `web/src/components/cliFormat.ts`
- Test: `web/src/components/cliFormat.test.ts`

**Interfaces:**
- Consumes: nothing.
- Produces (exact signatures — Tasks 3 and 8 import these):
  - `argSummary(args: unknown): string | null`
  - `resultSummary(content: string | undefined, resultStatus: string | undefined): string`
  - `blockMeter(pct: number): string`

- [ ] **Step 1: Write the failing tests**

Create `web/src/components/cliFormat.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { argSummary, resultSummary, blockMeter } from "./cliFormat";

describe("argSummary", () => {
  it("returns the first string value of an object arg", () => {
    expect(argSummary({ command: "npm test", cwd: "/x" })).toBe("npm test");
  });
  it("skips non-string values to find the first string", () => {
    expect(argSummary({ lines: 20, path: "web/src/state.ts" })).toBe("web/src/state.ts");
  });
  it("accepts a bare string arg", () => {
    expect(argSummary("ls -la")).toBe("ls -la");
  });
  it("uses only the first line of a multi-line value", () => {
    expect(argSummary({ script: "line one\nline two" })).toBe("line one");
  });
  it("truncates to 60 chars with an ellipsis", () => {
    const long = "x".repeat(80);
    const out = argSummary({ v: long })!;
    expect(out.length).toBe(60);
    expect(out.endsWith("…")).toBe(true);
  });
  it("returns null for empty object, arrays, numbers, and undefined", () => {
    expect(argSummary({})).toBeNull();
    expect(argSummary(["a"])).toBeNull();
    expect(argSummary(42)).toBeNull();
    expect(argSummary(undefined)).toBeNull();
    expect(argSummary({ v: "   " })).toBeNull();
  });
});

describe("resultSummary", () => {
  it("returns the first non-empty line", () => {
    expect(resultSummary("\n\n42 passed\n", "ok")).toBe("42 passed");
  });
  it("appends a line count when multi-line", () => {
    expect(resultSummary("a\nb\nc", "ok")).toBe("a (+2 lines)");
  });
  it("truncates the first line to 80 chars with an ellipsis", () => {
    const out = resultSummary("y".repeat(100), "ok");
    expect(out.length).toBe(80);
    expect(out.endsWith("…")).toBe(true);
  });
  it("says done for empty ok content and error for empty failed content", () => {
    expect(resultSummary("", "ok")).toBe("done");
    expect(resultSummary(undefined, "ok")).toBe("done");
    expect(resultSummary("  \n ", "error")).toBe("error");
  });
});

describe("blockMeter", () => {
  it("renders 10 cells, filled by tens", () => {
    expect(blockMeter(0)).toBe("░░░░░░░░░░");
    expect(blockMeter(60)).toBe("▂▂▂▂▂▂░░░░");
    expect(blockMeter(100)).toBe("▂▂▂▂▂▂▂▂▂▂");
  });
  it("clamps out-of-range values", () => {
    expect(blockMeter(-5)).toBe("░░░░░░░░░░");
    expect(blockMeter(140)).toBe("▂▂▂▂▂▂▂▂▂▂");
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `npx vitest run src/components/cliFormat.test.ts`
Expected: FAIL — cannot resolve `./cliFormat`.

- [ ] **Step 3: Implement `web/src/components/cliFormat.ts`**

```ts
const ARG_MAX = 60;
const RESULT_MAX = 80;

function truncate(s: string, max: number): string {
  return s.length > max ? s.slice(0, max - 1) + "…" : s;
}

function firstLine(s: string): string {
  return s.trim().split("\n")[0];
}

/** First string value in a tool's args, for the `Name(arg)` header line. */
export function argSummary(args: unknown): string | null {
  if (typeof args === "string" && args.trim() !== "") return truncate(firstLine(args), ARG_MAX);
  if (args && typeof args === "object" && !Array.isArray(args)) {
    for (const v of Object.values(args)) {
      if (typeof v === "string" && v.trim() !== "") return truncate(firstLine(v), ARG_MAX);
    }
  }
  return null;
}

/** One-line ⎿ summary of a tool result's raw content. */
export function resultSummary(content: string | undefined, resultStatus: string | undefined): string {
  const failed = !!resultStatus && resultStatus !== "ok";
  const lines = (content ?? "").split("\n").filter((l) => l.trim() !== "");
  if (lines.length === 0) return failed ? "error" : "done";
  const first = truncate(lines[0].trim(), RESULT_MAX);
  return lines.length > 1 ? `${first} (+${lines.length - 1} lines)` : first;
}

/** 10-cell context gauge: ▂ filled, ░ empty. */
export function blockMeter(pct: number): string {
  const filled = Math.max(0, Math.min(10, Math.round(pct / 10)));
  return "▂".repeat(filled) + "░".repeat(10 - filled);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/components/cliFormat.test.ts`
Expected: PASS (all 12).

- [ ] **Step 5: Commit**

```bash
git add src/components/cliFormat.ts src/components/cliFormat.test.ts
git commit -m "feat(web): cli formatting helpers (argSummary, resultSummary, blockMeter)"
```

---

### Task 3: Tool call → ⏺/⎿ transcript group

**Files:**
- Modify: `web/src/components/AnimatedToolCall.tsx` (full rewrite below)
- Test: `web/src/components/AnimatedToolCall.test.tsx`

**Interfaces:**
- Consumes: `argSummary`, `resultSummary` from `./cliFormat` (Task 2). Props are unchanged: `{ item, artifactKey?, active?, onSelect? }`.
- Produces: same component name/props — `MessageList.tsx` keeps using it as-is.

- [ ] **Step 1: Add failing tests**

Append these tests inside the existing `describe("AnimatedToolCall", …)` block in `web/src/components/AnimatedToolCall.test.tsx` (keep the two existing nesting tests unchanged — they must still pass):

```tsx
  it("shows the arg summary in the header", () => {
    render(<AnimatedToolCall item={toolItem({ name: "Bash", args: { command: "npm test" } })} />);
    expect(screen.getByText("(npm test)")).toBeInTheDocument();
  });
  it("renders a ⎿ result summary and expands raw content on click", () => {
    render(<AnimatedToolCall item={toolItem({ content: "42 passed\n0 failed" })} />);
    const summary = screen.getByText(/⎿ 42 passed \(\+1 lines\)/);
    expect(screen.queryByText(/0 failed/)).not.toBeInTheDocument();
    fireEvent.click(summary);
    expect(screen.getByText(/0 failed/)).toBeInTheDocument();
  });
  it("shows no ⎿ line while running", () => {
    render(<AnimatedToolCall item={toolItem({ status: "running" })} />);
    expect(screen.queryByText(/⎿/)).not.toBeInTheDocument();
  });
  it("marks failed results with the resultStatus and duration", () => {
    render(<AnimatedToolCall item={toolItem({ resultStatus: "error", durationMs: 42, content: "boom" })} />);
    expect(screen.getByText(/boom · error · 42ms/)).toBeInTheDocument();
  });
  it("calls onSelect from the view → affordance", () => {
    const onSelect = vi.fn();
    render(<AnimatedToolCall item={toolItem({})} artifactKey="art-1" onSelect={onSelect} />);
    fireEvent.click(screen.getByText("view →"));
    expect(onSelect).toHaveBeenCalledWith("art-1");
  });
```

Also update the test file's imports to:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
```

- [ ] **Step 2: Run tests to verify the new ones fail**

Run: `npx vitest run src/components/AnimatedToolCall.test.tsx`
Expected: the 2 existing tests PASS, the 5 new ones FAIL.

- [ ] **Step 3: Rewrite `web/src/components/AnimatedToolCall.tsx`**

```tsx
import { useState } from "react";
import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";
import { argSummary, resultSummary } from "./cliFormat";

interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
  /** Inspector key for this tool's artifact, if it produced one. */
  artifactKey?: string;
  /** True when this tool's artifact is the one open in the Inspector. */
  active?: boolean;
  onSelect?: (key: string) => void;
}

const EXPAND_LINES = 20;

// A tool call renders as a Claude Code-style transcript group:
//   ⏺ Name(arg-summary)
//     ⎿ result-summary        view →
// Clicking the ⎿ line toggles a raw-content preview (≤20 lines). `view →`
// focuses the Inspector artifact when the tool produced a display.
export function AnimatedToolCall({ item, artifactKey, active, onSelect }: Props) {
  const [expanded, setExpanded] = useState(false);
  const isRunning = item.status === "running";
  const failed = !!item.resultStatus && item.resultStatus !== "ok";
  const clickable = !!artifactKey && !!onSelect;
  // Attributed sub-agent tool rows nest under their dispatch parent: indent,
  // prefix a ↳, and strip the `sub:` display prefix from the tool name.
  const nested = !!item.parentId;
  const displayName = nested && item.name.startsWith("sub:") ? item.name.slice(4) : item.name;
  const arg = argSummary(item.args);
  const dot = isRunning ? "var(--cli-accent)" : failed ? "var(--cli-err)" : "var(--cli-ok)";
  const preview = (item.content ?? "").split("\n").slice(0, EXPAND_LINES).join("\n");

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5"
      style={{ marginLeft: nested ? "1.25rem" : undefined }}
    >
      <div className="flex items-baseline gap-2">
        {nested && <span style={{ color: "var(--cli-dim)" }}>↳</span>}
        {isRunning ? (
          <motion.span animate={{ opacity: [1, 0.3, 1] }} transition={{ repeat: Infinity, duration: 1.2 }}
            style={{ color: dot }}>⏺</motion.span>
        ) : (
          <span style={{ color: dot }}>⏺</span>
        )}
        <span style={{ color: "var(--cli-text)" }}>
          {displayName}
          {arg && <span style={{ color: "var(--cli-dim)" }}>({arg})</span>}
        </span>
      </div>
      {!isRunning && (
        <div className="flex items-baseline gap-2 pl-5">
          <button type="button" onClick={() => setExpanded((e) => !e)} className="text-left"
            style={{ color: failed ? "var(--cli-err)" : "var(--cli-dim)" }}>
            ⎿ {resultSummary(item.content, item.resultStatus)}
            {failed && ` · ${item.resultStatus} · ${item.durationMs}ms`}
          </button>
          {clickable && (
            <button type="button" onClick={() => onSelect!(artifactKey!)}
              style={{ color: "var(--cli-accent)" }}>
              {active ? "viewing →" : "view →"}
            </button>
          )}
        </div>
      )}
      {expanded && preview && (
        <pre className="mt-1 overflow-x-auto whitespace-pre-wrap pl-5"
          style={{ color: "var(--cli-dim)" }}>{preview}</pre>
      )}
    </motion.div>
  );
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/components/AnimatedToolCall.test.tsx`
Expected: PASS (7 tests: 2 existing nesting + 5 new).

- [ ] **Step 5: Full suite + typecheck, then commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS.

```bash
git add src/components/AnimatedToolCall.tsx src/components/AnimatedToolCall.test.tsx
git commit -m "feat(web): tool calls render as ⏺/⎿ transcript groups with inline previews"
```

---

### Task 4: Session banner, transcript lines, `cli` panel root

**Files:**
- Rename: `web/src/components/AgentHeader.tsx` → `web/src/components/SessionBanner.tsx` (git mv, new content below)
- Modify: `web/src/components/MessageList.tsx`
- Modify: `web/src/components/AgentColumn.tsx`
- Test: `web/src/components/AgentColumn.test.tsx`

**Interfaces:**
- Consumes: `AnimatedToolCall` (Task 3), `.cli` class (Task 1).
- Produces: `SessionBanner({ projectLabel, model }: { projectLabel: string; model?: string })`. `AgentColumn` props unchanged in this task (Task 6 adds `busy`/`turn`). `MessageList` props unchanged; it no longer scrolls itself (parent scrolls).

- [ ] **Step 1: Update `AgentColumn.test.tsx` expectations (failing first)**

In `web/src/components/AgentColumn.test.tsx`, replace the first test with a banner-only assertion (the Composer is untouched until Task 7, so its tests keep passing as-is):

```tsx
  it("renders the session banner (project + model)", () => {
    render(<AgentColumn {...base} />);
    expect(screen.getByText(/studio-x · qwen3/)).toBeInTheDocument();
  });
```

Leave the other three tests untouched (Task 7 updates the composer ones, Task 8 the dashboard one).

- [ ] **Step 2: Run to verify it fails**

Run: `npx vitest run src/components/AgentColumn.test.tsx`
Expected: the rewritten test FAILS (`studio-x · qwen3` not found — the old header renders "studio-x" and "model qwen3" separately); the other three still PASS.

- [ ] **Step 3: Create `SessionBanner`**

Run: `git mv src/components/AgentHeader.tsx src/components/SessionBanner.tsx`, then replace its content:

```tsx
// One-time transcript-top banner (replaces the old sticky AgentHeader).
export function SessionBanner({ projectLabel, model }: { projectLabel: string; model?: string }) {
  return (
    <div className="mx-4 my-3 rounded-md px-3 py-2"
      style={{ border: "1px solid var(--cli-border)", color: "var(--cli-dim)" }}>
      <span style={{ color: "var(--cli-accent)" }}>✻</span> {projectLabel}{model ? ` · ${model}` : ""}
    </div>
  );
}
```

- [ ] **Step 4: Restyle `MessageList.tsx` transcript lines**

Replace the root div's className and the `user`/`context` cases; full file:

```tsx
import type { AnimatedItem } from "../state";
import { AnimatedAssistantMessage } from "./AnimatedAssistantMessage";
import { AnimatedReasoningMessage } from "./AnimatedReasoningMessage";
import { AnimatedToolCall } from "./AnimatedToolCall";
import { AnimatedError } from "./AnimatedError";

export function MessageList({ items, activeArtifactKey, onSelectArtifact }:
  { items: AnimatedItem[]; activeArtifactKey?: string | null; onSelectArtifact?: (key: string) => void }) {
  return (
    <div className="px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 whitespace-pre-wrap" style={{ color: "var(--cli-dim)" }}>
              <span className="mr-2">&gt;</span>{it.text}</div>;
          case "assistant":
            return <AnimatedAssistantMessage key={i} item={it} />;
          case "reasoning":
            return <AnimatedReasoningMessage key={i} item={it} />;
          case "tool": {
            const artifactKey = it.display ? `art-${i}` : undefined;
            return <AnimatedToolCall key={i} item={it} artifactKey={artifactKey}
              active={!!artifactKey && artifactKey === activeArtifactKey} onSelect={onSelectArtifact} />;
          }
          case "context":
            return <div key={i} className="my-1" style={{ color: "var(--cli-dim)" }}>✻ {it.text}</div>;
          case "error":
            return <AnimatedError key={i} item={it} />;
        }
      })}
    </div>
  );
}
```

- [ ] **Step 5: Rewire `AgentColumn.tsx`**

Replace the import of `AgentHeader` with `SessionBanner` and restructure the root:

```tsx
import type { AnimatedItem, PendingApproval } from "../state";
import type { Decision, RuntimeSettings, SessionStats } from "../wire";
import { SessionBanner } from "./SessionBanner";
import { MessageList } from "./MessageList";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { ContextDashboard } from "./ContextDashboard";
import { Composer } from "./Composer";

export function AgentColumn({ items, activeArtifactKey, onSelectArtifact, projectLabel, model,
  pendingApproval, onDecide, composerDisabled, onSend, usage, settings, toolCount, artifactCount, stats }:
  { items: AnimatedItem[]; activeArtifactKey: string | null; onSelectArtifact: (key: string) => void;
    projectLabel: string; model?: string; pendingApproval: PendingApproval | null;
    onDecide: (d: Decision) => void; composerDisabled: boolean; onSend: (text: string) => void;
    usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number;
    stats: SessionStats | null }) {
  return (
    <div className="cli flex h-full min-h-0 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto py-2">
        <SessionBanner projectLabel={projectLabel} model={model} />
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
      </div>
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <ContextDashboard usage={usage} settings={settings} toolCount={toolCount} artifactCount={artifactCount} stats={stats} />
      <Composer disabled={composerDisabled} onSend={onSend} />
    </div>
  );
}
```

(Note: `background` now comes from `.cli`; the old inline `background: var(--surface-base)` is gone. Composer/status-line reordering happens in Task 8.)

- [ ] **Step 6: Run the full gate**

Run: `npx vitest run && npm run typecheck`
Expected: PASS — the banner test now finds `studio-x · qwen3`, and the untouched composer/dashboard tests still pass against the old Composer/ContextDashboard.

- [ ] **Step 7: Commit**

```bash
git add -A src/components src/index.css
git commit -m "feat(web): session banner + transcript-style user/context lines on cli panel root"
```

---

### Task 5: Assistant ⏺ prefix, ✻ Thinking, red error lines

**Files:**
- Modify: `web/src/components/AnimatedAssistantMessage.tsx`
- Modify: `web/src/components/AssistantMessage.tsx`
- Modify: `web/src/components/AnimatedReasoningMessage.tsx`
- Modify: `web/src/components/AnimatedError.tsx`

**Interfaces:**
- Consumes: `--cli-*` tokens (Task 1). All component names and props unchanged.
- Produces: nothing new for later tasks.

- [ ] **Step 1: Rewrite `AnimatedAssistantMessage.tsx`**

```tsx
import { motion } from "framer-motion";
import { useStreamingText } from "../hooks/useStreamingText";
import { MarkdownText } from "./MarkdownText";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "assistant" }>;
}

export function AnimatedAssistantMessage({ item }: Props) {
  const streaming = item.streaming && item.done === undefined;
  const visibleText = useStreamingText(item.text, streaming);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="flex gap-2 py-1.5"
      style={{ color: "var(--cli-text)" }}
    >
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1 whitespace-pre-wrap">
        <MarkdownText text={visibleText} />
        {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse" style={{ color: "var(--cli-accent)" }}>|</span>}
      </div>
    </motion.div>
  );
}
```

- [ ] **Step 2: Rewrite `AssistantMessage.tsx`**

```tsx
import type { Item } from "../state";
import { MarkdownText } from "./MarkdownText";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return (
    <div className="flex gap-2 py-1.5" style={{ color: "var(--cli-text)" }}>
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1"><MarkdownText text={item.text} /></div>
    </div>
  );
}
```

- [ ] **Step 3: Restyle `AnimatedReasoningMessage.tsx`**

```tsx
import { useState } from "react";
import { motion } from "framer-motion";
import { useStreamingText } from "../hooks/useStreamingText";
import { MarkdownText } from "./MarkdownText";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "reasoning" }>;
}

export function AnimatedReasoningMessage({ item }: Props) {
  const [open, setOpen] = useState(false);
  const streaming = item.streaming;
  const visibleText = useStreamingText(item.text, streaming);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5 italic"
      style={{ color: "var(--cli-dim)" }}
    >
      <button onClick={() => setOpen((o) => !o)} style={{ color: "var(--cli-dim)" }}>
        ✻ Thinking… {open ? "▾" : "▸"}
      </button>
      {open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
          className="pl-5"
        >
          <MarkdownText text={visibleText} />
          {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse" style={{ color: "var(--cli-accent)" }}>|</span>}
        </motion.div>
      )}
    </motion.div>
  );
}
```

- [ ] **Step 4: Restyle `AnimatedError.tsx`**

```tsx
import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "error" }>;
}

export function AnimatedError({ item }: Props) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      className="my-1.5 flex gap-2"
      style={{ color: "var(--cli-err)" }}
    >
      <span aria-hidden>⏺</span>
      <div className="min-w-0 flex-1 whitespace-pre-wrap">{item.message}</div>
    </motion.div>
  );
}
```

- [ ] **Step 5: Verify green and commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS.

```bash
git add src/components/AnimatedAssistantMessage.tsx src/components/AssistantMessage.tsx src/components/AnimatedReasoningMessage.tsx src/components/AnimatedError.tsx
git commit -m "feat(web): ⏺ assistant prefix, ✻ thinking, red ⏺ error lines"
```

---

### Task 6: Busy line (`✳ Verb… (Ns)`) wired from `state.inTurn`

**Files:**
- Create: `web/src/components/BusyLine.tsx`
- Test: `web/src/components/BusyLine.test.tsx`
- Modify: `web/src/components/AgentColumn.tsx` (add `busy`/`turn` props)
- Modify: `web/src/App.tsx:159-165` (pass the props)
- Test: `web/src/components/AgentColumn.test.tsx`

**Interfaces:**
- Consumes: `state.inTurn: boolean` and `state.turnIndex: number` (already in `ConversationState`, `web/src/state.ts:33` and `:32` — the reducer flips `inTurn` on `user_send`/`done`; no reducer changes).
- Produces: `BusyLine({ turn }: { turn: number })`, `busyVerb(turn: number): string`. `AgentColumn` gains required props `busy: boolean; turn: number`.

- [ ] **Step 1: Write failing tests**

Create `web/src/components/BusyLine.test.tsx`:

```tsx
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { BusyLine, busyVerb } from "./BusyLine";

describe("busyVerb", () => {
  it("cycles deterministically by turn", () => {
    expect(busyVerb(0)).toBe("Thinking");
    expect(busyVerb(1)).not.toBe(busyVerb(0));
    expect(busyVerb(6)).toBe(busyVerb(0));
  });
});

describe("BusyLine", () => {
  it("renders the spinner glyph, verb, and a seconds counter", () => {
    render(<BusyLine turn={0} />);
    expect(screen.getByText("✳")).toBeInTheDocument();
    expect(screen.getByText(/Thinking… \(0s\)/)).toBeInTheDocument();
  });
});
```

And in `AgentColumn.test.tsx`, add `busy: false, turn: 0,` to the `base` object and append:

```tsx
  it("shows the busy line while a turn is in flight", () => {
    render(<AgentColumn {...base} busy turn={0} />);
    expect(screen.getByText(/Thinking… \(0s\)/)).toBeInTheDocument();
  });
  it("hides the busy line when idle", () => {
    render(<AgentColumn {...base} />);
    expect(screen.queryByText("✳")).not.toBeInTheDocument();
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run src/components/BusyLine.test.tsx src/components/AgentColumn.test.tsx`
Expected: FAIL — module `./BusyLine` not found; AgentColumn has no `busy` prop.

- [ ] **Step 3: Implement `web/src/components/BusyLine.tsx`**

```tsx
import { useEffect, useState } from "react";

const VERBS = ["Thinking", "Wrangling", "Percolating", "Noodling", "Brewing", "Riffing"];

/** Deterministic per-turn verb so a turn keeps one verb for its lifetime. */
export function busyVerb(turn: number): string {
  return VERBS[Math.abs(turn) % VERBS.length];
}

// Claude Code-style working indicator: `✳ Verb… (Ns)`. The seconds counter
// starts when the line mounts (i.e. when the turn starts). No interrupt hint:
// the runtime has no cancel path.
export function BusyLine({ turn }: { turn: number }) {
  const [secs, setSecs] = useState(0);
  useEffect(() => {
    const id = setInterval(() => setSecs((s) => s + 1), 1000);
    return () => clearInterval(id);
  }, []);
  return (
    <div className="my-2 flex gap-2 px-4" style={{ color: "var(--cli-dim)" }}>
      <span className="animate-pulse" style={{ color: "var(--cli-accent)" }}>✳</span>
      <span>{busyVerb(turn)}… ({secs}s)</span>
    </div>
  );
}
```

- [ ] **Step 4: Wire it through `AgentColumn` and `App`**

In `AgentColumn.tsx`: add `busy, turn` to the destructured props, add `busy: boolean; turn: number;` to the prop types, import `BusyLine`, and render it after `MessageList` inside the scroll container:

```tsx
        <MessageList items={items} activeArtifactKey={activeArtifactKey} onSelectArtifact={onSelectArtifact} />
        {busy && <BusyLine turn={turn} />}
```

In `App.tsx`, add to the `<AgentColumn …>` call (around line 159):

```tsx
            busy={state.inTurn} turn={state.turnIndex}
```

- [ ] **Step 5: Run tests to verify they pass, full gate, commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS.

```bash
git add src/components/BusyLine.tsx src/components/BusyLine.test.tsx src/components/AgentColumn.tsx src/components/AgentColumn.test.tsx src/App.tsx
git commit -m "feat(web): ✳ busy line driven by state.inTurn with per-turn verbs"
```

---

### Task 7: Prompt-box composer with ↑/↓ history

**Files:**
- Modify: `web/src/components/Composer.tsx` (full rewrite)
- Create: `web/src/components/Composer.test.tsx`
- Modify: `web/src/components/AgentColumn.tsx` (pass `history`)
- Modify: `web/src/App.tsx` (provide the history getter)
- Test: `web/src/components/AgentColumn.test.tsx` (unskip Task 4's skips)

**Interfaces:**
- Consumes: `loadUserMsgs(sessionId: string): string[]` (`web/src/storage.ts:33`).
- Produces: `Composer({ disabled, onSend, history }: { disabled: boolean; onSend: (text: string) => void; history: () => string[] })`. `history` is a **getter** called on ArrowUp so it always sees the freshest persisted list (localStorage is written by `appendUserMsg` in `App.send` before dispatch). `AgentColumn` gains required prop `history: () => string[]`.

- [ ] **Step 1: Write failing tests**

Create `web/src/components/Composer.test.tsx`:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Composer } from "./Composer";

const setup = (history: string[] = []) => {
  const onSend = vi.fn();
  render(<Composer disabled={false} onSend={onSend} history={() => history} />);
  const ta = screen.getByRole("textbox", { name: "prompt" }) as HTMLTextAreaElement;
  return { onSend, ta };
};

describe("Composer", () => {
  it("sends on Enter and clears", () => {
    const { onSend, ta } = setup();
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello");
    expect(ta.value).toBe("");
  });
  it("does not send on Shift+Enter", () => {
    const { onSend, ta } = setup();
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter", shiftKey: true });
    expect(onSend).not.toHaveBeenCalled();
  });
  it("ArrowUp recalls history newest-first", () => {
    const { ta } = setup(["first", "second"]);
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("second");
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("first");
    fireEvent.keyDown(ta, { key: "ArrowUp" }); // at oldest: stays
    expect(ta.value).toBe("first");
  });
  it("ArrowDown walks forward and restores the draft past the newest", () => {
    const { ta } = setup(["first", "second"]);
    fireEvent.change(ta, { target: { value: "my draft" } });
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    fireEvent.keyDown(ta, { key: "ArrowDown" });
    expect(ta.value).toBe("my draft");
  });
  it("ArrowUp with no history is a no-op", () => {
    const { ta } = setup([]);
    fireEvent.change(ta, { target: { value: "draft" } });
    fireEvent.keyDown(ta, { key: "ArrowUp" });
    expect(ta.value).toBe("draft");
  });
  it("shows the disconnected placeholder when disabled", () => {
    render(<Composer disabled onSend={() => {}} history={() => []} />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run src/components/Composer.test.tsx`
Expected: FAIL — Composer has no `history` prop / no textbox named "prompt".

- [ ] **Step 3: Rewrite `web/src/components/Composer.tsx`**

```tsx
import { useRef, useState } from "react";

const MAX_ROWS = 6;
const ROW_PX = 22; // 13px * 1.65 line-height, rounded

// Claude Code-style prompt box: bordered, `>` prefix, Enter sends,
// Shift+Enter newlines, ↑/↓ walk the persisted prompt history.
export function Composer({ disabled, onSend, history }:
  { disabled: boolean; onSend: (text: string) => void; history: () => string[] }) {
  const [text, setText] = useState("");
  // null = editing a fresh draft; otherwise an index into history().
  const cursor = useRef<number | null>(null);
  const draft = useRef("");

  const submit = () => {
    const t = text.trim();
    if (!t || disabled) return;
    onSend(t);
    setText("");
    cursor.current = null;
  };

  const autogrow = (ta: HTMLTextAreaElement) => {
    ta.style.height = "auto";
    ta.style.height = `${Math.min(ta.scrollHeight, MAX_ROWS * ROW_PX)}px`;
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); return; }
    const ta = e.currentTarget;
    if (e.key === "ArrowUp" && !ta.value.slice(0, ta.selectionStart).includes("\n")) {
      const h = history();
      if (h.length === 0) return;
      e.preventDefault();
      if (cursor.current === null) { draft.current = text; cursor.current = h.length - 1; }
      else if (cursor.current > 0) { cursor.current -= 1; }
      setText(h[cursor.current]);
    } else if (e.key === "ArrowDown" && !ta.value.slice(ta.selectionEnd).includes("\n")) {
      if (cursor.current === null) return;
      const h = history();
      e.preventDefault();
      if (cursor.current < h.length - 1) { cursor.current += 1; setText(h[cursor.current]); }
      else { cursor.current = null; setText(draft.current); }
    }
  };

  return (
    <div className="p-3" style={{ borderTop: "1px solid var(--cli-border)" }}>
      <div className="cli-promptbox flex items-start gap-2 rounded-md px-3 py-2"
        style={{ border: "1px solid var(--cli-border)", opacity: disabled ? 0.5 : 1 }}>
        <span aria-hidden style={{ color: "var(--cli-dim)" }}>&gt;</span>
        <textarea
          aria-label="prompt"
          className="flex-1 resize-none bg-transparent outline-none disabled:opacity-50"
          style={{ color: "var(--cli-text)", font: "inherit", height: `${ROW_PX}px` }}
          rows={1}
          value={text}
          disabled={disabled}
          onChange={(e) => { setText(e.target.value); cursor.current = null; autogrow(e.target); }}
          onKeyDown={onKeyDown}
          placeholder={disabled ? "disconnected…" : ""}
        />
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Wire the history getter through**

In `AgentColumn.tsx`: add `history` to the destructured props, add `history: () => string[];` to the prop types, and pass it: `<Composer disabled={composerDisabled} onSend={onSend} history={history} />`.

In `App.tsx`: add `import { loadUserMsgs } from "./storage"` to the existing storage import line (it already imports `loadUserMsgs` — verify, it does at line 11), and pass on the `<AgentColumn …>` call:

```tsx
            history={() => loadUserMsgs(sessionId)}
```

In `AgentColumn.test.tsx`: add `history: () => [],` to `base`; extend the banner test with an enabled-composer assertion and update the two composer tests to the new selectors:

```tsx
  it("renders the session banner (project + model) and an enabled composer", () => {
    render(<AgentColumn {...base} />);
    expect(screen.getByText(/studio-x · qwen3/)).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "prompt" })).toBeEnabled();
  });
  it("disables the composer when asked", () => {
    render(<AgentColumn {...base} composerDisabled />);
    expect(screen.getByPlaceholderText(/disconnected/)).toBeDisabled();
  });
  it("sends a message", () => {
    const onSend = vi.fn();
    render(<AgentColumn {...base} onSend={onSend} />);
    const ta = screen.getByRole("textbox", { name: "prompt" });
    fireEvent.change(ta, { target: { value: "hello" } });
    fireEvent.keyDown(ta, { key: "Enter" });
    expect(onSend).toHaveBeenCalledWith("hello");
  });
```

- [ ] **Step 5: Run tests to verify they pass, full gate, commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS across the whole suite.

```bash
git add src/components/Composer.tsx src/components/Composer.test.tsx src/components/AgentColumn.tsx src/components/AgentColumn.test.tsx src/App.tsx
git commit -m "feat(web): bordered > prompt composer with ↑/↓ history, Send button removed"
```

---

### Task 8: Status line (block meter, below the prompt)

**Files:**
- Modify: `web/src/components/ContextDashboard.tsx`
- Modify: `web/src/components/AgentColumn.tsx` (reorder: Composer above ContextDashboard)
- Test: `web/src/components/AgentColumn.test.tsx`

**Interfaces:**
- Consumes: `blockMeter` from `./cliFormat` (Task 2). Props unchanged.
- Produces: nothing new.

- [ ] **Step 1: Extend the existing dashboard test (failing first)**

In `AgentColumn.test.tsx`, replace the "renders the context dashboard gauge" test with:

```tsx
  it("renders the status line with a block meter", () => {
    render(<AgentColumn {...base} usage={{ promptTokens: 4000, contextLimit: 8000, turn: 1, maxTurns: 20 }} />);
    expect(screen.getByLabelText("context usage")).toBeInTheDocument();
    expect(screen.getByText(/4k\s*\/\s*8k/)).toBeInTheDocument();
    expect(screen.getByText("▂▂▂▂▂░░░░░")).toBeInTheDocument();
    expect(screen.getByText(/50%/)).toBeInTheDocument();
    expect(screen.getByText(/turn 1\/20/)).toBeInTheDocument();
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run src/components/AgentColumn.test.tsx`
Expected: FAIL — no block-meter text.

- [ ] **Step 3: Rewrite `ContextDashboard.tsx`**

```tsx
import { useState } from "react";
import type { RuntimeSettings, SessionStats } from "../wire";
import { loadDashExpanded, saveDashExpanded } from "../storage";
import { StatsPanel } from "./StatsPanel";
import { blockMeter } from "./cliFormat";

function fmt(n: number): string {
  return n >= 1000 ? `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k` : `${n}`;
}

// Claude Code-style status line under the prompt box:
//   12.4k / 196k ▂▂▂░░░░░░░ 6% · qwen3.6 · turn 3/40        ▸
// Clicking toggles the expanded detail (model/temp, counts, skills, stats).
export function ContextDashboard(
  { usage, settings, toolCount, artifactCount, stats }:
  { usage: { promptTokens: number; contextLimit: number; turn: number; maxTurns: number } | null;
    settings: RuntimeSettings | null; toolCount: number; artifactCount: number;
    stats: SessionStats | null },
) {
  const [expanded, setExpanded] = useState(loadDashExpanded);
  const toggle = () => setExpanded((e) => { const next = !e; saveDashExpanded(next); return next; });

  const pct = usage ? Math.min(100, Math.round((usage.promptTokens / usage.contextLimit) * 100)) : 0;
  const over = pct >= 80;

  return (
    <div>
      <button onClick={toggle} aria-label="context usage" aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 pb-2 text-left"
        style={{ color: "var(--cli-dim)" }}>
        <span className="shrink-0">{usage ? `${fmt(usage.promptTokens)} / ${fmt(usage.contextLimit)}` : "— / —"}</span>
        <span aria-hidden className="shrink-0" style={{ color: over ? "var(--cli-err)" : "var(--cli-dim)" }}>
          {blockMeter(pct)}
        </span>
        <span className="shrink-0">{usage ? `${pct}%` : ""}</span>
        {settings && <span className="truncate">· {settings.model}</span>}
        {usage && <span className="shrink-0">· turn {usage.turn}/{usage.maxTurns}</span>}
        <span className="ml-auto shrink-0">{expanded ? "▾" : "▸"}</span>
      </button>

      {expanded && (
        <div className="space-y-1 px-3 pb-2" style={{ color: "var(--cli-dim)" }}>
          {settings && (
            <div>model {settings.model} · temp {settings.temperature}</div>
          )}
          {usage && (
            <div>turns {usage.turn}/{usage.maxTurns} · {toolCount} tools · {artifactCount} art</div>
          )}
          {settings && settings.active_skills.length > 0 && (
            <div>skills: {settings.active_skills.join(", ")}</div>
          )}
          <StatsPanel stats={stats} />
        </div>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Reorder in `AgentColumn.tsx`**

Swap the two lines so the status line sits below the prompt box:

```tsx
      {pendingApproval && <ApprovalPrompt approval={pendingApproval} onDecide={onDecide} />}
      <Composer disabled={composerDisabled} onSend={onSend} history={history} />
      <ContextDashboard usage={usage} settings={settings} toolCount={toolCount} artifactCount={artifactCount} stats={stats} />
```

- [ ] **Step 5: Run tests to verify they pass, full gate, commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS (StatsPanel and ContextDashboard-adjacent tests in `StatsPanel.test.tsx` still green — StatsPanel itself is untouched).

```bash
git add src/components/ContextDashboard.tsx src/components/AgentColumn.tsx src/components/AgentColumn.test.tsx
git commit -m "feat(web): status line with block meter below the prompt box"
```

---

### Task 9: Approval prompt — numbered options + 1/2/3 keys

**Files:**
- Modify: `web/src/components/ApprovalPrompt.tsx` (full rewrite)
- Create: `web/src/components/ApprovalPrompt.test.tsx`

**Interfaces:**
- Consumes: `Decision` = `"approve" | "approve_always" | "deny"` (as used by the current component), `PendingApproval` from `../state`. Props unchanged: `{ approval, onDecide }`.
- Produces: nothing new.

- [ ] **Step 1: Write failing tests**

Create `web/src/components/ApprovalPrompt.test.tsx`:

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ApprovalPrompt } from "./ApprovalPrompt";

const approval = { id: "a1", summary: "run `rm -rf node_modules`", command: "rm -rf node_modules" };

describe("ApprovalPrompt", () => {
  it("renders numbered options and decides on click", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.click(screen.getByText(/Yes, don't ask again/));
    expect(onDecide).toHaveBeenCalledWith("approve_always");
  });
  it("maps keys 1/2/3 to approve/approve_always/deny", () => {
    const onDecide = vi.fn();
    render(<ApprovalPrompt approval={approval} onDecide={onDecide} />);
    fireEvent.keyDown(window, { key: "3" });
    expect(onDecide).toHaveBeenCalledWith("deny");
  });
  it("ignores digit keys typed into a textarea", () => {
    const onDecide = vi.fn();
    render(
      <>
        <textarea aria-label="prompt" />
        <ApprovalPrompt approval={approval} onDecide={onDecide} />
      </>,
    );
    fireEvent.keyDown(screen.getByLabelText("prompt"), { key: "1" });
    expect(onDecide).not.toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npx vitest run src/components/ApprovalPrompt.test.tsx`
Expected: FAIL — no "Yes, don't ask again" text; keydown does nothing.

- [ ] **Step 3: Rewrite `web/src/components/ApprovalPrompt.tsx`**

```tsx
import { useEffect } from "react";
import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

const OPTIONS: { key: string; label: string; decision: Decision }[] = [
  { key: "1", label: "Yes", decision: "approve" },
  { key: "2", label: "Yes, don't ask again", decision: "approve_always" },
  { key: "3", label: "No", decision: "deny" },
];

// Claude Code-style permission box: numbered plain-text options, answerable
// with the 1/2/3 keys. Keystrokes originating in the composer (or any other
// text field) are ignored so typing digits never answers the approval.
export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target;
      if (t instanceof HTMLElement && (t.tagName === "TEXTAREA" || t.tagName === "INPUT" || t.isContentEditable)) return;
      const opt = OPTIONS.find((o) => o.key === e.key);
      if (opt) onDecide(opt.decision);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onDecide]);

  return (
    <div className="mx-4 my-2 rounded-md p-3" style={{ border: "1px solid var(--cli-border)" }}>
      <div className="mb-2" style={{ color: "var(--cli-text)" }}>Allow: {approval.summary}</div>
      {approval.command && (
        <pre className="mb-2 overflow-x-auto" style={{ color: "var(--cli-accent)" }}>{approval.command}</pre>
      )}
      <div className="flex flex-wrap gap-x-8 gap-y-1">
        {OPTIONS.map((o) => (
          <button key={o.key} type="button" onClick={() => onDecide(o.decision)}
            className="text-left hover:underline" style={{ color: "var(--cli-text)" }}>
            <span style={{ color: "var(--cli-dim)" }}>{o.key}.</span> {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `npx vitest run src/components/ApprovalPrompt.test.tsx`
Expected: PASS (3 tests).

- [ ] **Step 5: Full gate + commit**

Run: `npx vitest run && npm run typecheck`
Expected: PASS.

```bash
git add src/components/ApprovalPrompt.tsx src/components/ApprovalPrompt.test.tsx
git commit -m "feat(web): numbered approval prompt answerable with 1/2/3 keys"
```

---

### Task 10: Full verification gate + visual smoke check

**Files:**
- No new files. Fix-ups only if the gate finds issues.

**Interfaces:**
- Consumes: everything above.
- Produces: a green `scripts/ci.sh` and a visually confirmed panel.

- [ ] **Step 1: Full web suite**

Run: `cd /home/kalen/rust-agent-runtime/web && npx vitest run && npm run typecheck && npm run build`
Expected: all PASS, build clean.

- [ ] **Step 2: Repo CI gate**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh` (needs `source ~/.cargo/env` first if cargo is not on PATH)
Expected: fmt + clippy + cargo test + web typecheck/vitest all PASS (Rust untouched, so any Rust failure is pre-existing — report it, don't fix here).

- [ ] **Step 3: Visual smoke check (desktop app)**

Run: `cd /home/kalen/rust-agent-runtime && npm run desktop:dev`, send a prompt that triggers a tool call, and confirm against the spec: mono transcript in both themes (toggle via TopBar), `>` user line, ⏺/⎿ tool group with click-to-expand and `view →`, ✳ busy line while running, bordered prompt box with ↑ history, status line with block meter, numbered approval on a gated command. If anything diverges from the spec, fix and re-run Step 1.

- [ ] **Step 4: Commit any fix-ups**

```bash
git add -A && git commit -m "fix(web): cli panel polish from visual smoke check"
```

(Skip the commit if the tree is clean.)
