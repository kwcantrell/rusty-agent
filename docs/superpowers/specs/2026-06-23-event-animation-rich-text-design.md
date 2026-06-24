# Event Animation + Rich Text Display

**Date:** 2026-06-23
**Status:** Approved
**Scope:** Frontend only — no backend changes

## Overview

Bring the existing React+Vite+Tailwind web client to life with three animation layers and full markdown rendering:

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
| `rehype-pretty-code` | Syntax-highlighted code blocks (shiki) | ~15 KB |
| `remark-gfm` | GitHub Flavored Markdown (tables, strikethrough, task lists) | ~2 KB |

Total added bundle: ~57 KB (gzipped). Negligible for a dev tool.

## Animation System

### Layer 1: Streaming text effect

- `useStreamingText` hook tracks `isStreaming: boolean` per assistant/reasoning item
- Text renders character-by-character via `requestAnimationFrame` (not `setInterval`)
- Rate: ~60 chars/sec
- Blinking cursor at insertion point (CSS `@keyframes blink`)
- When item receives `done` signal, `isStreaming` flips to `false` and full text renders instantly
- Tool call status: running state shows pulsing ring (`framer-motion` `animate={{ scale: [1, 1.1, 1] }}` with `transition: { repeat: Infinity, duration: 1.5 }`)

### Layer 2: Message transitions

- Each message wrapped in `<motion.div>` with:
  - `initial={{ opacity: 0, y: 8 }}` → `animate={{ opacity: 1, y: 0 }}`
  - `exit={{ opacity: 0, height: 0 }}` (for reset/clear transitions)
  - `layout` prop on tool calls for smooth repositioning
- Stagger: 40ms between messages within same turn (`staggerChildren: 0.04` via `AnimatePresence`)

### Layer 3: Timeline view

- Horizontal scrollable bar, fixed height (~48px), below message list, above composer
- Turn segments: `[User pill] ──▶ (▸ Thinking) ──▶ [⚙ tool_call ···] ──▶ [✓ done]`
- Status colors: green=success, amber=running, red=error, blue=tool, purple=reasoning
- Connecting line: `h-px bg-zinc-700` between segments
- Interactivity:
  - Click turn segment → scrolls message list to that turn
  - Hover tool call bar → tooltip with name and duration
  - Hover thinking bar → tooltip with full reasoning text (truncated to 50 chars)
- Animated shimmer on running tool bars

## Rich Text Rendering

### MarkdownText component

- `react-markdown` with `remark-gfm` + `rehype-pretty-code` (shiki)
- Code blocks: dark theme with copy button on hover (top-right corner)
- Inline code: monospace, `bg-zinc-800 rounded px-1`
- Headings: h2/h3 size reduction for chat context
- Links: underlined, cyan color matching existing tool call style
- Lists: proper indentation

### Streaming integration

- Progressive markdown rendering during streaming (partial blocks show unclosed fences)
- On `done`, full markdown renders with complete syntax highlighting
- Code block copy button: `navigator.clipboard.writeText()`, shows "Copied!" tooltip

## New Types (additive, in state.ts)

```typescript
interface AnimatedItem extends Item {
  ts: number;        // timestamp when item was emitted (ms)
  streaming: boolean; // is this item still receiving events?
  progress: number;   // 0-1 for streaming text items
}
```

## useAnimatedItems Hook

Sits between `state.items` and render layer. Derives `AnimatedItem[]` from `Item[]` without mutating source of truth.

Responsibilities:
- Assigns timestamps to new items (Date.now() on emission)
- Tracks streaming state per item
- Groups items into turns (delimited by `done` events)
- Computes turn durations for timeline

## File Changes

| File | Change |
|------|--------|
| `web/package.json` | Add 4 deps |
| `web/src/state.ts` | Add `AnimatedItem` type + `useAnimatedItems` hook |
| `web/src/components/AnimatedAssistantMessage.tsx` | New — streaming + markdown |
| `web/src/components/AnimatedReasoningMessage.tsx` | New — streaming + collapse |
| `web/src/components/AnimatedToolCall.tsx` | New — expand/collapse + status pulse |
| `web/src/components/AnimatedError.tsx` | New — fade-in error |
| `web/src/components/TimelineView.tsx` | New — horizontal timeline |
| `web/src/components/MessageList.tsx` | Update — use animated components |
| `web/src/App.tsx` | Minimal — wire in TimelineView |

## Error Handling

- `react-markdown` handles malformed markdown gracefully (renders raw text)
- `rehype-pretty-code` falls back to plain text if syntax highlighting fails
- Streaming text: if websocket disconnects mid-stream, rAF loop stops and accumulated text renders
- Timeline: missing timestamps fall back to sequential ordering without duration bars

## Testing

- `useStreamingText` hook: unit test with mocked rAF — verify text accumulates and stops on `done`
- `AnimatedToolCall`: unit test — verify running→done animation sequence
- `TimelineView`: integration test — verify turn grouping produces correct segments
- Existing tests continue to pass (no changes to public component APIs)

## Backward Compatibility

- Existing `Item` type unchanged — all changes are additive
- `AnimatedItem` is a derived view, not persisted
- Old sessions (no timestamps) render correctly in both MessageList and TimelineView
- WebSocket protocol version unchanged
