# Event Animation + Rich Text Display

**Date:** 2026-06-23
**Status:** Approved
**Scope:** Frontend only — no backend changes

## Overview

Bring the existing React+Vite+Tailwind web client to life with three animation layers plus full markdown rendering:

1. **Streaming text** — assistant and reasoning text appears character-by-character
2. **Event transitions** — messages slide/fade in, tool calls pulse while running
3. **Timeline view** — horizontal scrollable bar showing turns, tool calls, and durations
4. **Rich text** — full markdown rendering with syntax-highlighted code blocks

## Architecture

```
App.tsx (unchanged wiring)
  ├── StatusBar (unchanged)
  ├── MessageList (updated: wraps items with animation context)
  │   ├── AnimatedAssistantMessage (new: streaming text + markdown)
  │   ├── AnimatedReasoningMessage (new: streaming + collapse)
  │   ├── AnimatedToolCall (new: expand/collapse + status pulse)
  │   └── AnimatedError (new: fade-in)
  ├── ApprovalPrompt (unchanged UI, subtle fade transition)
  ├── Composer (unchanged)
  ├── SettingsPanel (unchanged)
  └── TimelineView (new: horizontal event flow timeline)
```

No changes to the Rust backend or wire protocol. All changes are frontend-only.

## Dependencies (new)

| Package | Purpose | Size |
|---------|---------|------|
| `framer-motion` | All animations (streaming, transitions, timeline) | ~30 KB |
| `react-markdown` | Markdown rendering | ~10 KB |
| `rehype-highlight` | Syntax-highlighted code blocks (highlight.js, synchronous) | — |
| `remark-gfm` | GitHub Flavored Markdown (tables, strikethrough, task lists) | ~2 KB |

> **Highlighter note:** the original choice was `rehype-pretty-code` (shiki, theme `dark-plus`). It was dropped during implementation because shiki's rehype transform is **asynchronous**, while `react-markdown`'s default renderer is synchronous (`runSync`) — the combination throws `runSync finished async` on every render, in production as well as tests. `rehype-highlight` (highlight.js) is synchronous and works with `react-markdown` and with streaming re-renders; the theme is supplied by importing `highlight.js/styles/github-dark.css`. Actual production bundle is ~686 KB raw / ~213 KB gzipped (highlight.js bundles many language grammars, and framer-motion is sizeable) — heavier than first estimated, acceptable for a dev tool.

## Animation System

### Layer 1: Streaming text effect

- `useStreamingText` hook — signature: `useStreamingText(text: string, isStreaming: boolean): string`
  - Returns the visible text for rendering (full text when not streaming, partial when streaming)
  - Tracks the reveal index in a `ref`; resets to 0 whenever `text` or `isStreaming` changes
  - Reveal is **time-based**, not per-frame: on each rAF tick it reveals up to `floor(elapsed_seconds × 60)` characters, so it catches up correctly after dropped frames
- Rate: fixed at 60 chars/sec (`CHARS_PER_SECOND` module constant in `useStreamingText.ts`)
- Blinking cursor at insertion point — `useStreamingCursor` hook toggles a boolean every 530ms (`CURSOR_PERIOD_MS`) via `setInterval`; the component renders the cursor glyph based on that flag
- When `isStreaming` flips to `false`, the hook instantly returns the full text (no more rAF)
- Tool call status: running state shows pulsing ring (`framer-motion` `animate={{ scale: [1, 1.1, 1] }}` with `transition: { repeat: Infinity, duration: 1.5 }`)

### Layer 2: Message transitions

- Each message wrapped in `<motion.div>` with:
  - `initial={{ opacity: 0, y: 8 }}` → `animate={{ opacity: 1, y: 0 }}`
  - `exit={{ opacity: 0, height: 0 }}` (for reset/clear transitions)
  - `layout` prop on tool calls for smooth repositioning
- Stagger: 40ms between messages within same turn (`staggerChildren: 0.04` via `AnimatePresence`)
- `AnimatedError` uses the same pattern as other messages (fade-in via Layer 2); no special animation needed beyond the standard entry transition

### Layer 3: Timeline view

- Horizontal scrollable bar, fixed height (~48px), positioned between `MessageList` and `Composer` in the App layout
- Scroll-hint overlay: fades in on the left edge when content overflows
- Turn segments: `[User pill] ──▶ (▸ Thinking) ──▶ [⚙ tool_call ···] ──▶ [✓ done]`
- Status colors: green=success, amber=running, red=error, blue=tool, purple=reasoning
- Connecting line: `h-px bg-zinc-700` between segments
- Interactivity:
  - Click turn segment → scrolls message list to that turn
  - Hover tool call bar → tooltip with name (and duration, once real per-event timestamps land — see note below)
  - Hover thinking bar → tooltip with full reasoning text (truncated to 50 chars)

> **Timestamp note:** `animatedItemsFrom` currently assigns synthetic monotonic timestamps (`now, now+1, now+2 …`) as a stable ordering key, **not** wall-clock emission times. Turn "duration" derived from these equals `itemCount − 1` ms, so it is an ordering artifact, not a real duration. Real durations require the reducer to stamp `Date.now()` onto each item as it is emitted (a follow-up change to `reduceFrame`). Until then, the timeline renders segments and ordering but should not present duration as meaningful.
- Animated shimmer on running tool bars

## Rich Text Rendering

### MarkdownText component

- `react-markdown` with `remark-gfm` + `rehype-highlight` (highlight.js, `github-dark` theme CSS; synchronous so it works under react-markdown's `runSync` and during streaming re-renders)
- Code blocks: dark theme with copy button on hover (top-right corner)
- Inline code: monospace, `bg-zinc-800 rounded px-1`
- Headings: h2/h3 size reduction for chat context
- Links: underlined, cyan color matching existing tool call style
- Lists: proper indentation

### Streaming integration

- **As implemented:** the animated assistant/reasoning components render `MarkdownText` on the *progressively revealed* `useStreamingText` output, so markdown is re-parsed each animation tick. This is tolerable because `rehype-highlight` is synchronous (no async shiki tokenization); partial/unclosed fences render as plain text until the closing fence streams in.
- On `done` (item gains `done` / stops streaming), `useStreamingText` returns the full text and `MarkdownText` renders the complete, highlighted result.
- (A cheaper alternative — render plain text while streaming and only parse markdown once on `done` — was considered but not implemented; the per-tick sync re-parse is acceptable for a dev tool.)
- Code block copy button: `navigator.clipboard.writeText()`, shows "Copied!" tooltip

## New Types (additive, in state.ts)

```typescript
interface AnimatedItem extends Item {
  ts: number;        // timestamp when item was emitted (ms)
  streaming: boolean; // is this item still receiving events?
  progress: number;   // 0 while the item is still streaming, 1 once complete; per-character reveal progress is tracked in the component via useStreamingText, not here
}
```

## Deriving AnimatedItems

Pure functions in `state.ts` (`animatedItemsFrom`, `turnGroupsFrom`) sit between `state.items` and the render layer. They derive `AnimatedItem[]` / `TurnGroup[]` from `Item[]` without mutating the source of truth. (Not React hooks — plain functions, so they are trivially unit-testable; the test file is named `useAnimatedItems.test.ts` for historical reasons.)

Responsibilities:
- Assigns synthetic monotonic timestamps for stable ordering (the `now` base plus the item's index). Real wall-clock emission times are a future enhancement requiring the reducer to stamp items in `reduceFrame` — see the Timeline timestamp note above.
- Tracks streaming state per item
- Groups items into turns (delimited by `done` events)
- Computes turn durations for timeline

## File Changes

| File | Change |
|------|--------|
| `web/package.json` | Add 4 deps |
| `web/src/state.ts` | Add `AnimatedItem` type + `animatedItemsFrom`/`turnGroupsFrom` derive functions |
| `web/src/hooks/useStreamingText.ts` | New — `useStreamingText` + `useStreamingCursor` hooks |
| `web/src/components/AnimatedAssistantMessage.tsx` | New — streaming + markdown |
| `web/src/components/AnimatedReasoningMessage.tsx` | New — streaming + collapse |
| `web/src/components/AnimatedToolCall.tsx` | New — expand/collapse + status pulse |
| `web/src/components/AnimatedError.tsx` | New — fade-in error |
| `web/src/components/TimelineView.tsx` | New — horizontal timeline |
| `web/src/components/MessageList.tsx` | Update — use animated components |
| `web/src/App.tsx` | Minimal — wire in TimelineView |

## Error Handling

- `react-markdown` handles malformed markdown gracefully (renders raw text)
- `rehype-highlight` falls back to plain (unhighlighted) code if a language is unknown or highlighting fails
- Streaming text: if websocket disconnects mid-stream, rAF loop stops and accumulated text renders
- Timeline: missing timestamps fall back to sequential ordering without duration bars

## Testing

- `useStreamingText` hook: unit test with mocked rAF — verify text accumulates and stops on `done`
- `AnimatedToolCall`: unit test — verify running→done animation sequence
- `TimelineView`: integration test — verify turn grouping produces correct segments
- `MessageList` swaps in the `Animated*` components, which changes rendered markup. Existing `tool-components.test.tsx` / `shell-components.test.tsx` / `smoke.test.tsx` that assert on the current `ToolCall`/`AssistantMessage`/`ReasoningMessage` output will need updating in lockstep — the data flow into `MessageList` is unchanged, but the DOM it produces is not.

## Backward Compatibility

- Existing `Item` type unchanged — all type changes are additive
- `AnimatedItem` is a derived view, not persisted
- Old sessions (no timestamps) render correctly in both MessageList and TimelineView
- Note: "additive" applies to types and data flow, not to rendered DOM — see the Testing section for the component tests that change
- WebSocket protocol version unchanged
