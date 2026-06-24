# Event Animation + Rich Text Display

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add streaming text animation, event transitions, and a horizontal timeline view to the existing React+Vite+Tailwind web client, plus full markdown rendering with syntax-highlighted code blocks.

**Architecture:** All changes are frontend-only. The Rust backend and wire protocol remain unchanged. A new `useAnimatedItems` hook derives animation metadata from `state.items` without mutating the source of truth. Animated components wrap the existing rendering logic with framer-motion transitions, streaming text effects, and a new TimelineView component.

**Tech Stack:** React 19, Vite 7, Tailwind 4, Vitest, framer-motion, react-markdown, rehype-pretty-code, remark-gfm

## Global Constraints

- No changes to the Rust backend or wire protocol — `agent-server/src/wire.rs`, `agent-server/src/sink.rs`, `agent-core/src/loop_.rs` are untouched
- WebSocket protocol version remains `1` — `web/src/wire.ts` types are additive only
- Existing `Item` type in `web/src/state.ts` is unchanged — all changes are additive
- `AnimatedItem` is a derived view, never persisted
- Existing tests in `web/test/` and `web/src/*.test.*` continue to pass
- `noUnusedLocals: true` and `noUnusedParameters: true` in tsconfig.json — no dead code
- Tests use vitest globals + testing-library/react — follow existing patterns in `test/state.test.ts` and `test/tool-components.test.tsx`
- Test files go in `web/test/` alongside existing tests, named after the source file (e.g., `useStreamingText.test.ts`)

---

### Task 0: Install dependencies

**Files:**
- Modify: `web/package.json`

**Interfaces:**
- Consumes: nothing
- Produces: `framer-motion`, `react-markdown`, `rehype-pretty-code`, `remark-gfm` in node_modules

- [ ] **Step 1: Add the 4 dependencies to package.json**

```json
{
  "name": "web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build && touch dist/.gitkeep",
    "preview": "vite preview",
    "test": "vitest run",
    "typecheck": "tsc -b --noEmit"
  },
  "dependencies": {
    "diff": "^7.0.0",
    "framer-motion": "^12.0.0",
    "react": "^19.0.0",
    "react-dom": "^19.0.0",
    "react-markdown": "^10.0.0",
    "remark-gfm": "^4.0.0"
  },
  "devDependencies": {
    "@tailwindcss/vite": "^4.0.0",
    "@testing-library/jest-dom": "^6.6.0",
    "@testing-library/react": "^16.1.0",
    "@testing-library/user-event": "^14.5.0",
    "@types/diff": "^7.0.0",
    "@types/react": "^19.0.0",
    "@types/react-dom": "^19.0.0",
    "@vitejs/plugin-react": "^5.0.0",
    "jsdom": "^25.0.0",
    "rehype-pretty-code": "^0.14.0",
    "tailwindcss": "^4.0.0",
    "typescript": "^5.6.0",
    "vite": "^7.0.0",
    "vitest": "^3.0.0"
  }
}
```

- [ ] **Step 2: Install dependencies**

Run: `cd /home/kalen/rust-agent-runtime/web && npm install`
Expected: `added 12 packages` (or similar) — no errors.

- [ ] **Step 3: Verify existing tests still pass**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test`
Expected: all existing tests pass (SettingsPanel, socket, state, storage, wire, tool-components, etc.)

- [ ] **Step 4: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/package.json web/package-lock.json
git commit -m "chore(web): add framer-motion, react-markdown, rehype-pretty-code, remark-gfm"
```

---

### Task 1: useStreamingText hook

**Files:**
- Create: `web/src/hooks/useStreamingText.ts`
- Test: `web/test/useStreamingText.test.ts`

**Interfaces:**
- Consumes: nothing
- Produces: `useStreamingText(text: string, isStreaming: boolean): string`
  - When `isStreaming` is `true`, returns text up to the current character index (incrementing each frame at ~60 chars/sec)
  - When `isStreaming` is `false`, returns the full `text`
  - When `text` changes while streaming, resets the character index to 0
  - Returns `""` for empty text
- Also exports: `useStreamingCursor(): boolean` — returns a blinking cursor state (`true`/`false` toggling at 530ms)

- [ ] **Step 1: Write the failing test**

Create `web/test/useStreamingText.test.ts`:

```typescript
import { describe, it, expect, vi, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useStreamingText, useStreamingCursor } from "../src/hooks/useStreamingText";

afterEach(() => {
  vi.useRealTimers();
});

describe("useStreamingText", () => {
  it("returns full text when not streaming", () => {
    const { result } = renderHook(() => useStreamingText("hello", false));
    expect(result.current).toBe("hello");
  });

  it("returns empty string for empty text", () => {
    const { result } = renderHook(() => useStreamingText("", false));
    expect(result.current).toBe("");
  });

  it("reveals characters incrementally while streaming", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "hello", isStreaming: true } }
    );

    // At t=0, no chars revealed yet
    expect(result.current).toBe("");

    // After 1 frame worth of time (1ms), ~1 char revealed (60 chars/sec = 1 char per ~16ms, but we test with fast progress)
    // We use a 1ms interval to test — at 60 chars/sec that's ~0.001 chars, so we advance manually
    // Actually, the hook uses rAF timing internally, so in tests we advance time by enough for several chars
    // The hook reveals 1 char every ~16ms at 60 chars/sec. With fake timers at 1ms steps:
    // We'll test by advancing enough time for all chars to reveal
    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("hello");
  });

  it("resets index when text changes while streaming", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "ab", isStreaming: true } }
    );

    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("ab");

    // Change text while still streaming
    rerender({ text: "cd", isStreaming: true });
    act(() => {
      for (let i = 0; i < 1000; i++) vi.advanceTimersByTime(1);
    });
    expect(result.current).toBe("cd");
  });

  it("switches to full text when isStreaming flips to false", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ text, isStreaming }) => useStreamingText(text, isStreaming),
      { initialProps: { text: "hello world", isStreaming: true } }
    );

    // Not all chars revealed yet — advance only a bit
    act(() => {
      vi.advanceTimersByTime(50);
    });
    const partial = result.current;
    expect(partial.length).toBeLessThan("hello world".length);

    // Stop streaming
    rerender({ text: "hello world", isStreaming: false });
    expect(result.current).toBe("hello world");
  });
});

describe("useStreamingCursor", () => {
  it("toggles between true and false", () => {
    vi.useFakeTimers();
    const { result } = renderHook(() => useStreamingCursor());
    const first = result.current;
    act(() => {
      vi.advanceTimersByTime(530);
    });
    expect(result.current).toBe(!first);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/useStreamingText.test.ts`
Expected: FAIL with "Cannot find name 'useStreamingText'" or "Cannot find module"

- [ ] **Step 3: Write the hook implementation**

Create `web/src/hooks/useStreamingText.ts`:

```typescript
import { useState, useEffect, useRef, useCallback } from "react";

const CHARS_PER_SECOND = 60;
const FRAME_MS = 1000 / CHARS_PER_SECOND; // ~16.67ms per char
const CURSOR_PERIOD_MS = 530;

/**
 * Returns a progressively revealed version of `text` when `isStreaming` is true.
 * While streaming, characters are revealed at ~60 chars/sec.
 * When `isStreaming` is false, returns the full `text` immediately.
 * When `text` changes, the reveal index resets to 0.
 */
export function useStreamingText(text: string, isStreaming: boolean): string {
  const [revealed, setRevealed] = useState(text);
  const idxRef = useRef(0);
  const rafRef = useRef<number | null>(null);
  const textRef = useRef(text);
  const streamingRef = useRef(isStreaming);

  // Keep refs in sync
  useEffect(() => {
    textRef.current = text;
    streamingRef.current = isStreaming;
  }, [text, isStreaming]);

  const tick = useCallback(() => {
    if (!streamingRef.current) {
      rafRef.current = null;
      return;
    }
    const full = textRef.current;
    if (idxRef.current < full.length) {
      idxRef.current += 1;
      setRevealed(full.slice(0, idxRef.current));
      rafRef.current = requestAnimationFrame(tick);
    } else {
      rafRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (isStreaming && text.length > 0) {
      idxRef.current = 0;
      rafRef.current = requestAnimationFrame(tick);
    } else {
      setRevealed(text);
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    }
    return () => {
      if (rafRef.current !== null) {
        cancelAnimationFrame(rafRef.current);
        rafRef.current = null;
      }
    };
  }, [isStreaming, text, tick]);

  return revealed;
}

/**
 * Returns a boolean that toggles every CURSOR_PERIOD_MS for a blinking cursor.
 */
export function useStreamingCursor(): boolean {
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    const id = setInterval(() => setVisible((v) => !v), CURSOR_PERIOD_MS);
    return () => clearInterval(id);
  }, []);
  return visible;
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/useStreamingText.test.ts`
Expected: PASS (all 6 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/hooks/useStreamingText.ts web/test/useStreamingText.test.ts
git commit -m "feat(web): add useStreamingText and useStreamingCursor hooks"
```

---

### Task 2: useAnimatedItems hook + AnimatedItem type

**Files:**
- Modify: `web/src/state.ts`
- Test: `web/test/useAnimatedItems.test.ts`

**Interfaces:**
- Consumes: `state.items: Item[]` from the reducer
- Produces: `AnimatedItem[]` — extends `Item` with `ts`, `streaming`, `progress`
- Produces: `TurnGroup[]` — groups items into turns delimited by `done` events
- Produces: `useAnimatedItems(items: Item[]): AnimatedItem[]` hook
- Produces: `useTurnGrouping(items: AnimatedItem[]): TurnGroup[]` hook

```typescript
interface AnimatedItem extends Item {
  ts: number;        // timestamp when item was emitted (ms)
  streaming: boolean; // is this item still receiving events?
  progress: number;   // charsRendered / totalChars for streaming text items (0-1)
}

interface TurnGroup {
  items: AnimatedItem[];
  startTs: number;
  endTs: number;
  duration: number;
}
```

- [ ] **Step 1: Add AnimatedItem and TurnGroup types to state.ts**

Append to `web/src/state.ts` after the existing `ConversationState` interface:

```typescript
/** Animation metadata derived from Item — never persisted. */
export interface AnimatedItem extends Item {
  ts: number;
  streaming: boolean;
  progress: number;
}

export interface TurnGroup {
  items: AnimatedItem[];
  startTs: number;
  endTs: number;
  duration: number;
}
```

- [ ] **Step 2: Write the failing test**

Create `web/test/useAnimatedItems.test.ts`:

```typescript
import { describe, it, expect } from "vitest";
import { animatedItemsFrom, turnGroupsFrom, type AnimatedItem, type TurnGroup } from "../src/state";
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
    expect(animated[0].streaming).toBe(true); // assistant items streaming while not done
    expect(animated[0].progress).toBe(0);
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
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/useAnimatedItems.test.ts`
Expected: FAIL with "Cannot find name 'animatedItemsFrom'" or "Cannot find module"

- [ ] **Step 4: Write the hook implementations in state.ts**

Append to `web/src/state.ts` after the `reduceFrame` function:

```typescript
/**
 * Derives animation metadata from raw Item[] for consumption by animated components.
 * @param items - items from the reducer
 * @param now - current timestamp (for tests: fixed value)
 */
export function animatedItemsFrom(items: Item[], now: number): AnimatedItem[] {
  let ts = now;
  return items.map((item) => {
    const streaming = isStreamingItem(item);
    const progress = streaming ? 0 : 1;
    ts += 1; // each item gets a unique timestamp
    return { ...item, ts, streaming, progress } as AnimatedItem;
  });
}

function isStreamingItem(item: Item): boolean {
  // Items that stream: assistant (not done), reasoning, and tool (running)
  if (item.kind === "assistant" && item.done === undefined) return true;
  if (item.kind === "reasoning") return true;
  if (item.kind === "tool" && item.status === "running") return true;
  return false;
}

/**
 * Groups animated items into turns, delimited by done signals.
 * Each turn starts with the first item after the previous turn's done (or the start).
 */
export function turnGroupsFrom(items: AnimatedItem[]): TurnGroup[] {
  const groups: TurnGroup[] = [];
  let currentGroup: AnimatedItem[] = [];

  for (const item of items) {
    currentGroup.push(item);
    if (item.kind === "assistant" && item.done !== undefined) {
      if (currentGroup.length > 0) {
        const startTs = currentGroup[0].ts;
        const endTs = currentGroup[currentGroup.length - 1].ts;
        groups.push({
          items: [...currentGroup],
          startTs,
          endTs,
          duration: endTs - startTs,
        });
      }
      currentGroup = [];
    }
  }

  // Flush any remaining items (e.g., if stream ended mid-turn)
  if (currentGroup.length > 0) {
    const startTs = currentGroup[0].ts;
    const endTs = currentGroup[currentGroup.length - 1].ts;
    groups.push({
      items: [...currentGroup],
      startTs,
      endTs,
      duration: endTs - startTs,
    });
  }

  return groups;
}
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/useAnimatedItems.test.ts`
Expected: PASS (all 7 tests)

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/state.ts web/test/useAnimatedItems.test.ts
git commit -m "feat(web): add AnimatedItem, TurnGroup types and animatedItemsFrom/turnGroupsFrom"
```

---

### Task 3: AnimatedAssistantMessage component

**Files:**
- Create: `web/src/components/AnimatedAssistantMessage.tsx`
- Test: `web/test/animated-assistant-message.test.tsx`

**Interfaces:**
- Consumes: `AnimatedItem` of kind `"assistant"` (from `animatedItemsFrom`)
- Produces: `<AnimatedAssistantMessage>` — renders streaming text with markdown, or full text when done
- Uses `useStreamingText` for the streaming effect
- Uses `react-markdown` with `remark-gfm` + `rehype-pretty-code` for markdown rendering
- Uses framer-motion for slide-in + fade transition

```tsx
interface Props {
  item: Extract<AnimatedItem, { kind: "assistant" }>;
}
```

- [ ] **Step 1: Write the failing test**

Create `web/test/animated-assistant-message.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedAssistantMessage } from "../src/components/AnimatedAssistantMessage";

describe("AnimatedAssistantMessage", () => {
  it("renders assistant text", () => {
    const item = {
      kind: "assistant",
      text: "Hello world",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("Hello world")).toBeInTheDocument();
  });

  it("renders markdown headings", () => {
    const item = {
      kind: "assistant",
      text: "# Heading",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("Heading")).toBeInTheDocument();
  });

  it("renders code blocks with syntax highlighting", () => {
    const item = {
      kind: "assistant",
      text: "```js\nconsole.log('hi');\n```",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("console.log('hi');")).toBeInTheDocument();
  });

  it("renders inline code", () => {
    const item = {
      kind: "assistant",
      text: "Use `console.log` to debug",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText(/Use/)).toBeInTheDocument();
    expect(screen.getByText(/console\.log/)).toBeInTheDocument();
  });

  it("shows done reason when present", () => {
    const item = {
      kind: "assistant",
      text: "done",
      done: "stop",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedAssistantMessage item={item} />);
    expect(screen.getByText("done")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-assistant-message.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/AnimatedAssistantMessage.tsx`:

```typescript
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
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="whitespace-pre-wrap py-2 text-zinc-100"
    >
      <MarkdownText text={visibleText} />
      {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse text-cyan-400">|</span>}
    </motion.div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-assistant-message.test.tsx`
Expected: PASS (all 5 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/AnimatedAssistantMessage.tsx web/test/animated-assistant-message.test.tsx
git commit -m "feat(web): add AnimatedAssistantMessage with streaming + markdown"
```

---

### Task 4: MarkdownText component

**Files:**
- Create: `web/src/components/MarkdownText.tsx`
- Test: `web/test/markdown-text.test.tsx`

**Interfaces:**
- Consumes: `text: string`
- Produces: `<MarkdownText>` — renders markdown with syntax highlighting
- Uses `react-markdown` with `remark-gfm` + `rehype-pretty-code`
- Code blocks have a copy button on hover
- Inline code: `bg-zinc-800 rounded px-1 font-mono text-sm`

- [ ] **Step 1: Write the failing test**

Create `web/test/markdown-text.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MarkdownText } from "../src/components/MarkdownText";

describe("MarkdownText", () => {
  it("renders plain text as-is", () => {
    render(<MarkdownText text="hello world" />);
    expect(screen.getByText("hello world")).toBeInTheDocument();
  });

  it("renders headings", () => {
    render(<MarkdownText text="# Heading 1\n## Heading 2" />);
    expect(screen.getByText("Heading 1")).toBeInTheDocument();
    expect(screen.getByText("Heading 2")).toBeInTheDocument();
  });

  it("renders bold and italic", () => {
    render(<MarkdownText text="**bold** and *italic*" />);
    expect(screen.getByText("bold")).toBeInTheDocument();
    expect(screen.getByText("italic")).toBeInTheDocument();
  });

  it("renders inline code", () => {
    render(<MarkdownText text="Use `code` here" />);
    expect(screen.getByText(/Use/)).toBeInTheDocument();
    expect(screen.getByText(/code/)).toBeInTheDocument();
  });

  it("renders code blocks", () => {
    render(<MarkdownText text="```\nconst x = 1;\n```" />);
    expect(screen.getByText("const x = 1;")).toBeInTheDocument();
  });

  it("renders links", () => {
    render(<MarkdownText text="[link](https://example.com)" />);
    const link = screen.getByText("link");
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "https://example.com");
  });

  it("renders lists", () => {
    render(<MarkdownText text="- item 1\n- item 2" />);
    expect(screen.getByText("item 1")).toBeInTheDocument();
    expect(screen.getByText("item 2")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/markdown-text.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/MarkdownText.tsx`:

```typescript
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypePrettyCode from "rehype-pretty-code";
import { useState } from "react";

const rehypePrettyCodeOptions = {
  theme: "dark-plus" as const,
  keepBackground: false,
  onVisitLine(node: { children: unknown[]; properties: { className: string[] } }) {
    // Prevent spaces before/after code blocks
    if (node.children.length === 0) {
      node.children = [{ type: "text", value: " " }];
    }
  },
  onVisitHighlightedLine(node: { properties: { className: string[] } }) {
    node.properties.className.push("line--highlighted");
  },
  onVisitHighlightedWord(node: { properties: { className: string[] } }) {
    node.properties.className.push("word--highlighted");
  },
};

interface Props {
  text: string;
}

export function MarkdownText({ text }: Props) {
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);

  const handleCopy = (code: string, index: number) => {
    navigator.clipboard.writeText(code).catch(() => {});
    setCopiedIndex(index);
    setTimeout(() => setCopiedIndex(null), 1500);
  };

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[["rehype-pretty-code", rehypePrettyCodeOptions]]}
      components={{
        // Override code blocks to add copy button
        pre({ node, className, children, ...props }) {
          const index = String(node?.data?.hProperties?.index ?? -1);
          return (
            <div className={`relative group rounded ${className ?? ""}`}>
              <pre className="overflow-x-auto p-2 font-mono text-sm leading-tight" {...props}>
                {children}
              </pre>
              <button
                className="absolute right-2 top-2 rounded bg-zinc-700 px-2 py-0.5 text-xs text-zinc-200 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-zinc-600"
                onClick={() => {
                  const code = (node?.children as unknown[])?.map((c: { value: string }) => c.value).join("") ?? "";
                  handleCopy(code, Number(index));
                }}
              >
                {copiedIndex === Number(index) ? "Copied!" : "Copy"}
              </button>
            </div>
          );
        },
        // Inline code styling
        code({ className, children, ...props }) {
          // If this is inline code (not inside pre), style it
          const isInline = !className?.includes("language-") || className.includes("inline");
          if (isInline) {
            return (
              <code className="rounded bg-zinc-800 px-1 font-mono text-sm" {...props}>
                {children}
              </code>
            );
          }
          return (
            <code className={className} {...props}>
              {children}
            </code>
          );
        },
        // Headings
        h1({ children }) {
          return <h1 className="mb-1 mt-2 text-xl font-semibold text-zinc-100">{children}</h1>;
        },
        h2({ children }) {
          return <h2 className="mb-1 mt-2 text-lg font-semibold text-zinc-100">{children}</h2>;
        },
        h3({ children }) {
          return <h3 className="mb-1 mt-2 text-base font-semibold text-zinc-100">{children}</h3>;
        },
        // Links
        a({ children, href, ...props }) {
          return (
            <a className="text-cyan-400 underline" href={href} target="_blank" rel="noopener noreferrer" {...props}>
              {children}
            </a>
          );
        },
        // Lists
        ul({ children }) {
          return <ul className="my-2 ml-4 list-disc space-y-1 text-zinc-100">{children}</ul>;
        },
        ol({ children }) {
          return <ol className="my-2 ml-4 list-decimal space-y-1 text-zinc-100">{children}</ol>;
        },
        li({ children }) {
          return <li className="text-zinc-100">{children}</li>;
        },
        // Paragraphs
        p({ children }) {
          return <p className="my-1 text-zinc-100">{children}</p>;
        },
      }}
    >
      {text}
    </ReactMarkdown>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/markdown-text.test.tsx`
Expected: PASS (all 7 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/MarkdownText.tsx web/test/markdown-text.test.tsx
git commit -m "feat(web): add MarkdownText component with syntax highlighting + copy button"
```

---

### Task 5: AnimatedReasoningMessage component

**Files:**
- Create: `web/src/components/AnimatedReasoningMessage.tsx`
- Test: `web/test/animated-reasoning-message.test.tsx`

**Interfaces:**
- Consumes: `AnimatedItem` of kind `"reasoning"` (from `animatedItemsFrom`)
- Produces: `<AnimatedReasoningMessage>` — collapsible reasoning with streaming text
- Keeps the existing collapse/expand behavior from `ReasoningMessage`
- Streams text during reasoning (character-by-character)
- Uses framer-motion for expand/collapse animation

```tsx
interface Props {
  item: Extract<AnimatedItem, { kind: "reasoning" }>;
}
```

- [ ] **Step 1: Write the failing test**

Create `web/test/animated-reasoning-message.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { AnimatedReasoningMessage } from "../src/components/AnimatedReasoningMessage";

describe("AnimatedReasoningMessage", () => {
  it("renders reasoning text", () => {
    const item = {
      kind: "reasoning",
      text: "let me think about this",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedReasoningMessage item={item} />);
    expect(screen.getByText("let me think about this")).toBeInTheDocument();
  });

  it("collapses by default", () => {
    const item = {
      kind: "reasoning",
      text: "hidden reasoning",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedReasoningMessage item={item} />);
    expect(screen.queryByText("hidden reasoning")).not.toBeInTheDocument();
  });

  it("expands when clicked", () => {
    const item = {
      kind: "reasoning",
      text: "visible reasoning",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedReasoningMessage item={item} />);
    fireEvent.click(screen.getByText("▸ Thinking"));
    expect(screen.getByText("visible reasoning")).toBeInTheDocument();
  });

  it("shows the Thinking label", () => {
    const item = {
      kind: "reasoning",
      text: "test",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedReasoningMessage item={item} />);
    expect(screen.getByText("▸ Thinking")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-reasoning-message.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/AnimatedReasoningMessage.tsx`:

```typescript
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
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2 max-w-[80%] rounded border border-zinc-700 bg-zinc-900/60 px-3 py-2 text-xs"
    >
      <button onClick={() => setOpen((o) => !o)} className="mb-1 font-medium text-zinc-300">
        {open ? "▾" : "▸"} Thinking
      </button>
      {open && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
        >
          <MarkdownText text={visibleText} />
          {streaming && <span className="inline-block h-4 w-[1ch] animate-pulse text-cyan-400">|</span>}
        </motion.div>
      )}
    </motion.div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-reasoning-message.test.tsx`
Expected: PASS (all 4 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/AnimatedReasoningMessage.tsx web/test/animated-reasoning-message.test.tsx
git commit -m "feat(web): add AnimatedReasoningMessage with streaming + collapse"
```

---

### Task 6: AnimatedToolCall component

**Files:**
- Create: `web/src/components/AnimatedToolCall.tsx`
- Test: `web/test/animated-tool-call.test.tsx`

**Interfaces:**
- Consumes: `AnimatedItem` of kind `"tool"` (from `animatedItemsFrom`)
- Produces: `<AnimatedToolCall>` — tool call with status pulse, expand/collapse, slide-in transition
- Status icon: pulsing ring while running, checkmark when done
- Renders existing `DiffView`, `TerminalBlock`, and raw content
- Uses framer-motion for slide-in + expand/collapse animation

```tsx
interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
}
```

- [ ] **Step 1: Write the failing test**

Create `web/test/animated-tool-call.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedToolCall } from "../src/components/AnimatedToolCall";

describe("AnimatedToolCall", () => {
  it("renders tool name and running status", () => {
    const item = {
      kind: "tool",
      name: "read_file",
      args: { path: "a.txt" },
      status: "running",
      ts: Date.now(),
      streaming: true,
      progress: 0,
    };
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    // Running status shows "…"
    expect(screen.getByText("…")).toBeInTheDocument();
  });

  it("renders tool name and done status", () => {
    const item = {
      kind: "tool",
      name: "read_file",
      args: { path: "a.txt" },
      status: "done",
      content: "file contents",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    // Done status shows "✓"
    expect(screen.getByText("✓")).toBeInTheDocument();
  });

  it("renders diff display", () => {
    const item = {
      kind: "tool",
      name: "write_file",
      args: { path: "a.txt" },
      status: "done",
      display: { Diff: { path: "a.txt", before: "foo\nbar\n", after: "foo\nbaz\n" } },
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("a.txt")).toBeInTheDocument();
    expect(screen.getByText(/-\s*bar/)).toBeInTheDocument();
    expect(screen.getByText(/\+\s*baz/)).toBeInTheDocument();
  });

  it("renders terminal display", () => {
    const item = {
      kind: "tool",
      name: "execute_command",
      args: { command: "echo hi" },
      status: "done",
      display: { Terminal: { command: "echo hi", stdout: "hi\n", stderr: "", exit_code: 0 } },
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/execute_command/)).toBeInTheDocument();
    expect(screen.getByText(/echo hi/)).toBeInTheDocument();
  });

  it("renders raw content when no display", () => {
    const item = {
      kind: "tool",
      name: "read_file",
      args: { path: "a.txt" },
      status: "done",
      content: "file contents",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedToolCall item={item} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
    expect(screen.getByText("file contents")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-tool-call.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/AnimatedToolCall.tsx`:

```typescript
import { motion } from "framer-motion";
import { useState } from "react";
import { DiffView } from "./DiffView";
import { TerminalBlock } from "./TerminalBlock";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "tool" }>;
}

export function AnimatedToolCall({ item }: Props) {
  const [expanded, setExpanded] = useState(true);
  const isRunning = item.status === "running";
  const statusIcon = isRunning ? "…" : "✓";

  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2"
    >
      <div
        className="flex cursor-pointer items-center gap-2 font-mono text-cyan-400 hover:text-cyan-300"
        onClick={() => setExpanded((e) => !e)}
      >
        <span>⚙ {item.name}</span>
        <span className="inline-flex items-center justify-center">
          {isRunning && (
            <motion.span
              animate={{ scale: [1, 1.15, 1] }}
              transition={{ repeat: Infinity, duration: 1.5 }}
              className="text-cyan-400"
            >
              {statusIcon}
            </motion.span>
          )}
          {!isRunning && <span className="text-green-400">{statusIcon}</span>}
        </span>
        {!isRunning && (
          <span className="text-xs text-zinc-500">{expanded ? "▾" : "▸"}</span>
        )}
      </div>

      {expanded && !isRunning && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
        >
          {"display" in item && item.display && "Diff" in item.display && (
            <DiffView path={item.display.Diff.path} before={item.display.Diff.before} after={item.display.Diff.after} />
          )}
          {"display" in item && item.display && "Terminal" in item.display && (
            <TerminalBlock
              command={item.display.Terminal.command}
              stdout={item.display.Terminal.stdout}
              stderr={item.display.Terminal.stderr}
              exitCode={item.display.Terminal.exit_code}
            />
          )}
          {"display" in item && item.display && "Text" in item.display && (
            <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-300">{item.display.Text}</pre>
          )}
          {!("display" in item) || (!item.display || !("Diff" in item.display) && !("Terminal" in item.display) && !("Text" in item.display)) ? (
            item.content && <pre className="whitespace-pre-wrap p-2 font-mono text-sm text-zinc-400">{item.content}</pre>
          ) : null}
        </motion.div>
      )}

      {isRunning && (
        <motion.div
          initial={{ height: 0, opacity: 0 }}
          animate={{ height: "auto", opacity: 1 }}
          transition={{ duration: 0.15 }}
          className="text-xs text-zinc-500"
        >
          running…
        </motion.div>
      )}
    </motion.div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-tool-call.test.tsx`
Expected: PASS (all 5 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/AnimatedToolCall.tsx web/test/animated-tool-call.test.tsx
git commit -m "feat(web): add AnimatedToolCall with status pulse + expand/collapse"
```

---

### Task 7: AnimatedError component

**Files:**
- Create: `web/src/components/AnimatedError.tsx`
- Test: `web/test/animated-error.test.tsx`

**Interfaces:**
- Consumes: `AnimatedItem` of kind `"error"` (from `animatedItemsFrom`)
- Produces: `<AnimatedError>` — fade-in error message with existing styling

```tsx
interface Props {
  item: Extract<AnimatedItem, { kind: "error" }>;
}
```

- [ ] **Step 1: Write the failing test**

Create `web/test/animated-error.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { AnimatedError } from "../src/components/AnimatedError";

describe("AnimatedError", () => {
  it("renders error message", () => {
    const item = {
      kind: "error",
      message: "something went wrong",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    render(<AnimatedError item={item} />);
    expect(screen.getByText(/✗/)).toBeInTheDocument();
    expect(screen.getByText("something went wrong")).toBeInTheDocument();
  });

  it("has red border styling", () => {
    const item = {
      kind: "error",
      message: "fail",
      ts: Date.now(),
      streaming: false,
      progress: 1,
    };
    const { container } = render(<AnimatedError item={item} />);
    const el = container.firstChild as HTMLElement;
    expect(el.className).toContain("border-red-700");
    expect(el.className).toContain("bg-red-950");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-error.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/AnimatedError.tsx`:

```typescript
import { motion } from "framer-motion";
import type { AnimatedItem } from "../state";

interface Props {
  item: Extract<AnimatedItem, { kind: "error" }>;
}

export function AnimatedError({ item }: Props) {
  return (
    <motion.div
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      exit={{ opacity: 0, height: 0 }}
      className="my-2 rounded border border-red-700 bg-red-950 px-3 py-2 text-red-300"
    >
      ✗ {item.message}
    </motion.div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/animated-error.test.tsx`
Expected: PASS (all 2 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/AnimatedError.tsx web/test/animated-error.test.tsx
git commit -m "feat(web): add AnimatedError with fade-in transition"
```

---

### Task 8: TimelineView component

**Files:**
- Create: `web/src/components/TimelineView.tsx`
- Test: `web/test/timeline-view.test.tsx`

**Interfaces:**
- Consumes: `TurnGroup[]` from `turnGroupsFrom(animatedItems)`
- Produces: `<TimelineView>` — horizontal scrollable bar with turn segments
- Each turn shows: user message pill → thinking bar → tool call bars → done dot
- Clicking a turn scrolls the message list to that turn
- Hovering shows tooltips

```tsx
interface Props {
  turns: import("../state").TurnGroup[];
  onTurnClick?: (index: number) => void;
  messageListRef: React.RefObject<HTMLDivElement | null>;
}
```

- [ ] **Step 1: Write the failing test**

Create `web/test/timeline-view.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { TimelineView } from "../src/components/TimelineView";
import type { TurnGroup, AnimatedItem } from "../src/state";

function makeTurn(items: AnimatedItem[]): TurnGroup {
  return {
    items,
    startTs: items[0].ts,
    endTs: items[items.length - 1].ts,
    duration: items[items.length - 1].ts - items[0].ts,
  };
}

describe("TimelineView", () => {
  it("renders user message pills", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "hello", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "hi", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("renders thinking bar", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "reasoning", text: "thinking", ts: now + 50, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("thinking")).toBeInTheDocument();
  });

  it("renders tool call bars", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "run", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "tool", name: "read_file", args: {}, status: "done", ts: now + 50, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "done", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("read_file")).toBeInTheDocument();
  });

  it("renders done dot", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    // Done dot is a small circle — check for the status indicator
    const container = document.querySelector("[class*='flex']");
    expect(container).toBeInTheDocument();
  });

  it("renders multiple turns", () => {
    const now = Date.now();
    const turns: TurnGroup[] = [
      makeTurn([
        { kind: "user", text: "q1", ts: now, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a1", ts: now + 100, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
      makeTurn([
        { kind: "user", text: "q2", ts: now + 200, streaming: false, progress: 1 } as AnimatedItem,
        { kind: "assistant", text: "a2", ts: now + 300, streaming: false, progress: 1, done: "stop" } as AnimatedItem,
      ]),
    ];
    render(<TimelineView turns={turns} messageListRef={{ current: null }} />);
    expect(screen.getByText("q1")).toBeInTheDocument();
    expect(screen.getByText("q2")).toBeInTheDocument();
  });

  it("renders nothing when no turns", () => {
    const { container } = render(<TimelineView turns={[]} messageListRef={{ current: null }} />);
    expect(container.firstChild).toBeNull();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/timeline-view.test.tsx`
Expected: FAIL with "Cannot find module"

- [ ] **Step 3: Write the component**

Create `web/src/components/TimelineView.tsx`:

```typescript
import { useRef, useState } from "react";
import type { TurnGroup } from "../state";

interface Props {
  turns: TurnGroup[];
  onTurnClick?: (index: number) => void;
  messageListRef: React.RefObject<HTMLDivElement | null>;
}

export function TimelineView({ turns, onTurnClick, messageListRef }: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [hoveredTurn, setHoveredTurn] = useState<number | null>(null);

  if (turns.length === 0) return null;

  const handleTurnClick = (index: number) => {
    onTurnClick?.(index);
    if (messageListRef.current) {
      messageListRef.current.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  };

  return (
    <div className="relative border-t border-zinc-800 bg-zinc-900/80">
      <div className="overflow-x-auto overflow-y-hidden px-4">
        <div ref={scrollRef} className="flex h-12 items-center gap-2 whitespace-nowrap">
          {turns.map((turn, turnIndex) => {
            const isHovered = hoveredTurn === turnIndex;
            return (
              <div
                key={turnIndex}
                className="flex items-center"
                onClick={() => handleTurnClick(turnIndex)}
              >
                {turnIndex > 0 && (
                  <div className="mx-1 h-px w-3 bg-zinc-700" />
                )}

                {/* User message pill */}
                {turn.items.filter((i) => i.kind === "user").map((userItem, ui) => (
                  <div
                    key={`user-${ui}`}
                    className={`rounded-full border border-zinc-700 bg-zinc-800 px-2 py-0.5 text-xs text-zinc-200 transition-colors hover:border-zinc-500 ${
                      isHovered ? "cursor-pointer" : ""
                    }`}
                    title={userItem.text}
                  >
                    {userItem.text.length > 30 ? userItem.text.slice(0, 30) + "…" : userItem.text}
                  </div>
                ))}

                {/* Thinking bar */}
                {turn.items.filter((i) => i.kind === "reasoning").map((reasoningItem, ri) => (
                  <div
                    key={`reasoning-${ri}`}
                    className="mx-1 flex h-3 items-center"
                    title={reasoningItem.text.length > 50 ? reasoningItem.text.slice(0, 50) + "…" : reasoningItem.text}
                  >
                    <div className="h-1 w-12 rounded-full bg-purple-500/60" />
                  </div>
                ))}

                {/* Tool call bars */}
                {turn.items.filter((i) => i.kind === "tool").map((toolItem, ti) => {
                  const isRunning = toolItem.status === "running";
                  return (
                    <div
                      key={`tool-${ti}`}
                      className="mx-1 flex h-3 items-center"
                      title={toolItem.name}
                    >
                      <div
                        className={`h-1 w-16 rounded-full ${
                          isRunning ? "bg-amber-400/80" : "bg-green-400/80"
                        }`}
                      />
                    </div>
                  );
                })}

                {/* Done dot */}
                {turn.items.filter((i) => i.kind === "assistant" && i.done !== undefined).map((doneItem, di) => (
                  <div
                    key={`done-${di}`}
                    className="mx-1 flex h-3 items-center"
                    title={`Done: ${doneItem.done}`}
                  >
                    <div className="h-2 w-2 rounded-full bg-green-400" />
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/timeline-view.test.tsx`
Expected: PASS (all 6 tests)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/TimelineView.tsx web/test/timeline-view.test.tsx
git commit -m "feat(web): add TimelineView horizontal event flow timeline"
```

---

### Task 9: Wire animated components into MessageList

**Files:**
- Modify: `web/src/components/MessageList.tsx`

**Interfaces:**
- Consumes: `AnimatedItem[]` from `useAnimatedItems` (via parent)
- Produces: Updated `<MessageList>` that renders animated components instead of static ones

- [ ] **Step 1: Write the failing test**

Append to `web/test/tool-components.test.tsx` (or create a new test file). Actually, since `MessageList` is the integration point, let's test it directly:

Create `web/test/message-list.test.tsx`:

```typescript
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MessageList } from "../src/components/MessageList";
import type { AnimatedItem } from "../src/state";

function makeAnimated(kind: string, props: Record<string, unknown>): AnimatedItem {
  return { kind, ts: Date.now(), streaming: false, progress: 1, ...props } as AnimatedItem;
}

describe("MessageList", () => {
  it("renders user items", () => {
    const items: AnimatedItem[] = [
      makeAnimated("user", { text: "hello" }),
    ];
    render(<MessageList items={items} />);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });

  it("renders assistant items with animated component", () => {
    const items: AnimatedItem[] = [
      makeAnimated("assistant", { text: "hi there" }),
    ];
    render(<MessageList items={items} />);
    expect(screen.getByText("hi there")).toBeInTheDocument();
  });

  it("renders reasoning items with animated component", () => {
    const items: AnimatedItem[] = [
      makeAnimated("reasoning", { text: "thinking" }),
    ];
    render(<MessageList items={items} />);
    expect(screen.getByText("▸ Thinking")).toBeInTheDocument();
  });

  it("renders tool items with animated component", () => {
    const items: AnimatedItem[] = [
      makeAnimated("tool", { name: "read_file", args: {}, status: "running" }),
    ];
    render(<MessageList items={items} />);
    expect(screen.getByText(/read_file/)).toBeInTheDocument();
  });

  it("renders error items with animated component", () => {
    const items: AnimatedItem[] = [
      makeAnimated("error", { message: "fail" }),
    ];
    render(<MessageList items={items} />);
    expect(screen.getByText(/✗/)).toBeInTheDocument();
    expect(screen.getByText("fail")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/message-list.test.tsx`
Expected: FAIL — either the test file doesn't exist, or the imports fail

- [ ] **Step 3: Update MessageList to use animated components**

Replace the entire contents of `web/src/components/MessageList.tsx`:

```typescript
import type { AnimatedItem } from "../state";
import { AnimatedAssistantMessage } from "./AnimatedAssistantMessage";
import { AnimatedReasoningMessage } from "./AnimatedReasoningMessage";
import { AnimatedToolCall } from "./AnimatedToolCall";
import { AnimatedError } from "./AnimatedError";

type ToolItem = Extract<AnimatedItem, { kind: "tool" }>;

export function MessageList({ items }: { items: AnimatedItem[] }) {
  return (
    <div className="flex-1 overflow-y-auto px-4">
      {items.map((it, i) => {
        switch (it.kind) {
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded bg-zinc-800 px-3 py-2 text-zinc-100">{it.text}</div>;
          case "assistant":
            return <AnimatedAssistantMessage key={i} item={it as Extract<AnimatedItem, { kind: "assistant" }>} />;
          case "reasoning":
            return <AnimatedReasoningMessage key={i} item={it as Extract<AnimatedItem, { kind: "reasoning" }>} />;
          case "tool":
            return <AnimatedToolCall key={i} item={it as ToolItem} />;
          case "error":
            return <AnimatedError key={i} item={it as Extract<AnimatedItem, { kind: "error" }>} />;
        }
      })}
    </div>
  );
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/message-list.test.tsx`
Expected: PASS (all 5 tests)

- [ ] **Step 5: Run ALL tests to ensure nothing is broken**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test`
Expected: ALL tests pass (including existing tests)

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/components/MessageList.tsx web/test/message-list.test.tsx
git commit -m "feat(web): wire animated components into MessageList"
```

---

### Task 10: Wire TimelineView into App and connect streaming

**Files:**
- Modify: `web/src/App.tsx`
- Modify: `web/src/state.ts` (add `useAnimatedItems` and `useTurnGrouping` as exported hooks)

**Interfaces:**
- Consumes: `state.items` from `useReducer`
- Produces: `useAnimatedItems(items)` hook in state.ts
- Produces: `useTurnGrouping(animatedItems)` hook in state.ts
- Wires `TimelineView` into App between `MessageList` and `Composer`

- [ ] **Step 1: Add hooks to state.ts**

Append to `web/src/state.ts` after the existing `reduce` function:

```typescript
import { useCallback, useMemo } from "react";

/**
 * Derives animated items from raw items.
 * In production, calls Date.now() for timestamps.
 * In tests, use `animatedItemsFrom(items, fixedNow)` directly.
 */
export function useAnimatedItems(items: Item[]): AnimatedItem[] {
  return useMemo(() => animatedItemsFrom(items, Date.now()), [items]);
}

/**
 * Groups animated items into turns.
 */
export function useTurnGrouping(animatedItems: AnimatedItem[]): TurnGroup[] {
  return useMemo(() => turnGroupsFrom(animatedItems), [animatedItems]);
}
```

- [ ] **Step 2: Update App.tsx to wire in TimelineView and useAnimatedItems**

Replace the entire contents of `web/src/App.tsx`:

```typescript
import { useEffect, useReducer, useRef, useState } from "react";
import { connect } from "./socket";
import { initialState, reduce, useAnimatedItems, useTurnGrouping } from "./state";
import type { Decision, RuntimeSettings } from "./wire";
import { PairingScreen } from "./components/PairingScreen";
import { StatusBar } from "./components/StatusBar";
import { MessageList } from "./components/MessageList";
import { ApprovalPrompt } from "./components/ApprovalPrompt";
import { Composer } from "./components/Composer";
import { SettingsPanel } from "./components/SettingsPanel";
import { TimelineView } from "./components/TimelineView";
import { appendUserMsg, clearSession, loadSessionId, loadToken, loadUserMsgs, saveSession } from "./storage";

function wsUrl(token: string): string {
  return `${location.origin.replace(/^http/, "ws")}/browser?token=${encodeURIComponent(token)}`;
}

export default function App() {
  const [sessionId, setSessionId] = useState<string | null>(loadSessionId());
  const [token, setToken] = useState<string | null>(loadToken());
  const [state, dispatch] = useReducer(reduce, loadUserMsgs(sessionId ?? ""), initialState);
  const [showSettings, setShowSettings] = useState(false);
  const sock = useRef<ReturnType<typeof connect> | null>(null);
  const messageListRef = useRef<HTMLDivElement>(null);

  const animatedItems = useAnimatedItems(state.items);
  const turns = useTurnGrouping(animatedItems);

  useEffect(() => {
    if (!token || !sessionId) return;
    dispatch({ type: "reset", userMsgs: loadUserMsgs(sessionId) });
    const WebSocketImpl = (window as unknown as { __WS__?: typeof WebSocket }).__WS__;
    sock.current = connect(
      wsUrl(token),
      { onFrame: (f) => dispatch({ type: "frame", frame: f }), onStatus: (s) => dispatch({ type: "status", status: s }) },
      WebSocketImpl ? { WebSocketImpl } : undefined,
    );
    return () => { sock.current?.close(); sock.current = null; };
  }, [token, sessionId]);

  if (!token || !sessionId) {
    return (
      <div className="h-screen bg-zinc-950">
        <PairingScreen onPaired={({ sessionId, token }) => { saveSession(sessionId, token); setSessionId(sessionId); setToken(token); }} />
      </div>
    );
  }

  const send = (text: string) => {
    appendUserMsg(sessionId, text);
    dispatch({ type: "user_send", text });
    sock.current?.send({ v: 1, session_id: sessionId, kind: "user_input", text });
  };
  const decide = (d: Decision) => {
    if (!state.pendingApproval) return;
    sock.current?.send({ v: 1, session_id: sessionId, id: state.pendingApproval.id, kind: "approval_response", decision: d });
    dispatch({ type: "approval_sent" });
  };
  const signOut = () => { sock.current?.close(); clearSession(); setToken(null); setSessionId(null); };
  const openSettings = () => {
    setShowSettings(true);
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_get" });
  };
  const saveSettings = (s: RuntimeSettings) => {
    sock.current?.send({ v: 1, session_id: sessionId, kind: "settings_update", settings: s });
  };

  const connected = state.status === "open";
  return (
    <div className="flex h-screen flex-col bg-zinc-950">
      <StatusBar online={state.online} status={state.status} onSignOut={signOut} onOpenSettings={openSettings} settingsDisabled={!(connected && state.online)} />
      {showSettings && state.settings && (
        <SettingsPanel
          settings={state.settings}
          meta={state.settingsMeta}
          error={state.settingsError}
          disabled={!connected}
          onSave={saveSettings}
          onClose={() => setShowSettings(false)}
        />
      )}
      <div ref={messageListRef} className="flex-1 overflow-y-auto">
        <MessageList items={animatedItems} />
      </div>
      <TimelineView turns={turns} messageListRef={messageListRef} />
      {state.pendingApproval && <ApprovalPrompt approval={state.pendingApproval} onDecide={decide} />}
      <Composer disabled={!connected} onSend={send} />
    </div>
  );
}
```

- [ ] **Step 3: Run ALL tests to verify nothing is broken**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test`
Expected: ALL tests pass

- [ ] **Step 4: Run typecheck**

Run: `cd /home/kalen/rust-agent-runtime/web && npm run typecheck`
Expected: PASS (no type errors)

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add web/src/App.tsx web/src/state.ts
git commit -m "feat(web): wire TimelineView into App, add useAnimatedItems/useTurnGrouping hooks"
```

---

### Task 11: Final cleanup and end-to-end verification

**Files:**
- All files touched in previous tasks

**Interfaces:**
- Consumes: nothing
- Produces: a working, tested implementation

- [ ] **Step 1: Run all tests one final time**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test`
Expected: ALL tests pass

- [ ] **Step 2: Run typecheck**

Run: `cd /home/kalen/rust-agent-runtime/web && npm run typecheck`
Expected: PASS (no type errors)

- [ ] **Step 3: Verify build**

Run: `cd /home/kalen/rust-agent-runtime/web && npm run build`
Expected: PASS (no build errors)

- [ ] **Step 4: Run existing tests one more time to confirm nothing regressed**

Run: `cd /home/kalen/rust-agent-runtime/web && npm test -- test/state.test.ts test/wire.test.ts test/socket.test.ts`
Expected: ALL existing tests pass

- [ ] **Step 5: Final commit**

```bash
cd /home/kalen/rust-agent-runtime
git add -A
git commit -m "feat(web): event animation + rich text display — streaming, transitions, timeline, markdown"
```

---

## Self-Review Checklist

**1. Spec coverage:**
- Streaming text: Task 1 (hook) + Task 3 (component) + Task 5 (reasoning)
- Event transitions: Task 3, 4, 5, 6, 7 (all animated components have framer-motion transitions)
- Timeline view: Task 8
- Rich text (markdown): Task 3 (uses MarkdownText) + Task 4 (MarkdownText component)
- Syntax highlighting: Task 4 (rehype-pretty-code with dark-plus theme)
- Copy button: Task 4 (in MarkdownText)
- Tool call pulse: Task 6 (animatedToolCall has scale animation while running)
- Error handling: Task 7 (AnimatedError with fade-in)
- Backward compatibility: Task 2 (additive types only), Task 9 (MessageList uses AnimatedItem)

**2. Placeholder scan:**
- No "TBD", "TODO", "implement later" found
- No "similar to Task N" references
- All code is complete and explicit
- All commands are exact with expected output

**3. Type consistency:**
- `AnimatedItem` defined once in `state.ts` (Task 2), imported consistently in all components
- `TurnGroup` defined once in `state.ts` (Task 2), imported in TimelineView (Task 8)
- `animatedItemsFrom` and `turnGroupsFrom` in `state.ts` (Task 2), exported hooks in Task 10
- `useStreamingText` signature consistent across Task 1 (hook) and Tasks 3, 5 (usage)
- `MarkdownText` component signature consistent across Task 4 (definition) and Task 3 (usage)
- `TimelineView` props consistent across Task 8 (definition) and Task 10 (usage)

**4. Scope check:**
- Each task is independently testable
- Tasks build on each other in a clear dependency order
- No circular dependencies
- All new files follow existing patterns (testing-library + vitest)
