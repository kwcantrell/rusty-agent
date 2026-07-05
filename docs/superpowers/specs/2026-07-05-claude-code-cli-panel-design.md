# Claude Code–style CLI panel

**Date:** 2026-07-05
**Status:** Approved design, pending implementation plan
**Scope:** `web/` only — no Rust, wire-protocol, or right-pane changes.

## Goal

Restyle the left agent panel (`AgentColumn` and children) so it looks and feels
like Claude Code's terminal UI: a monospace transcript with `>` user lines,
`⏺ Tool(args)` rows with inline `⎿` result previews, a bordered `>` prompt box,
and a status line — in both light and dark themes, with Claude's coral accent.

Decisions made during brainstorming:

- **Full look & feel** — adopt Claude Code's transcript grammar and inline tool
  results, not just a reskin. The Inspector remains the home for full artifacts;
  the transcript gains short result previews.
- **Both themes, Claude accent** — the panel follows the app's light/dark
  toggle; both variants get terminal treatment plus the coral accent. New
  tokens are scoped to the panel so the right pane is untouched.
- **Frontend-only behaviors** — ↑/↓ prompt history and expandable `⎿` previews.
  No interrupt: the wire protocol has no cancel message, so no
  "esc to interrupt" hint is shown anywhere.
- **Approach A** — restyle the existing component tree in place. No parallel
  component tree, no toggle, no terminal emulator.

## Visual identity

All new styling hangs off a `cli` class on the panel root. Tokens are defined
per theme in `src/index.css` under the existing `:root[data-theme=…]` blocks:

| token | dark | light | used for |
|-------|------|-------|----------|
| `--cli-bg` | `#1a1915` | `#faf9f5` | panel background |
| `--cli-text` | `#e8e6dc` | `#3d3d38` | assistant text, assistant `⏺` |
| `--cli-dim` | `#8c8a7d` | `#8a877c` | user lines, `⎿` results, thinking, status line |
| `--cli-accent` | `#d97757` | `#c15f3c` | running `⏺`, `✳` spinner, prompt focus border, `view →` |
| `--cli-ok` | `#6fae72` | `#4f7a52` | completed tool `⏺` |
| `--cli-err` | `#e0654f` | `#b3402e` | failed tool `⏺`, error lines |
| `--cli-border` | `#35332c` | `#e3e0d5` | prompt box, approval box, session banner |

Typography: the whole panel uses a monospace face. Add
`@fontsource-variable/jetbrains-mono` (imported in `src/main.tsx`), exposed as
`--font-cli` with fallback `ui-monospace, "SF Mono", monospace`. Base size
13px, line-height 1.65. Markdown inside assistant messages keeps its
structure (bold, lists, code spans) but inherits mono. Fraunces/Inter remain
in the TopBar and right pane — the panel reads as a console embedded in the
app, a deliberate material change.

The signature element is the transcript grammar itself (`>`, `⏺`, `⎿`, `✳` in
coral). Everything else stays quiet: flat surfaces, no shadows, no bubbles.
Entrance animation is a fast opacity fade only (no y-slide); the running-dot
pulse stays; `prefers-reduced-motion` is already handled globally.

## Transcript rendering

The sticky `AgentHeader` is removed. The transcript instead opens with a
one-time **session banner** — a thin `--cli-border` box as the first scroll
item: `✻ session 4f3a91c2 · qwen3.6` in dim mono. (Session and model also
appear in the TopBar and status line.)

Item kinds, all left-aligned with ~6px vertical rhythm:

- **User** — `>` prefix + text, both `--cli-dim`, `whitespace-pre-wrap`.
- **Assistant** — `⏺` in `--cli-text` with the markdown body in a hanging
  indent (wrapped lines align under the text, not the glyph). Streaming
  behavior unchanged.
- **Thinking** — `✻ Thinking…` header, dim italic; click toggles the body
  (dim italic markdown). Collapsed by default as today; streaming caret
  recolored to `--cli-accent`.
- **Tool call** — two-line group replacing the chip:
  - Header: `⏺ Name(arg-summary)`. Glyph is coral + pulsing while running,
    `--cli-ok` on success, `--cli-err` on failure. Arg summary = first string
    value found in `args`, truncated to 60 chars; unparseable or empty args →
    bare name, no parens. Sub-agent rows keep `↳` + indent and the stripped
    `sub:` prefix, as today.
  - Result: `⎿ result-summary` in dim — first non-empty line of `content`,
    truncated to 80 chars, with `(+N lines)` appended when multi-line.
    Clicking toggles an expanded dim `pre` of up to 20 lines of raw content.
    Empty content → `⎿ done`, or `⎿ error` in `--cli-err` when
    `resultStatus ≠ "ok"` (failed rows also show `resultStatus · durationMs`
    as today). If the tool carries a `display` artifact, a coral `view →` at
    the end of the ⎿ line focuses the Inspector (existing select behavior,
    relocated).
- **Context events** — left-aligned dim line `✻ <text>` (replaces the
  centered `· text ·`).
- **Errors** — red `⏺` + message in `--cli-err`.
- **Busy line** — while a turn is in flight, the transcript's last line is
  `✳ <Verb>… (Ns)`, ✳ in coral, with a per-turn verb from a small rotation
  (e.g. Thinking, Wrangling, Percolating). In-flight is derived at render
  time from existing state (any tool `status === "running"` or a streaming
  assistant/reasoning item) — no reducer changes. The seconds counter starts
  when the busy line mounts.

## Composer, status line, approval prompt

**Composer** (`Composer.tsx`) — one rounded box, 1px `--cli-border` border
(coral on focus), dim `>` prefix glyph, transparent mono textarea auto-growing
from 1 to ~6 lines. Enter sends, Shift+Enter newlines. The Send button is
removed; Enter is the only send path. Disabled: dimmed box, placeholder
`disconnected…`.

**Prompt history** — ↑ with the caret on the first line (or empty box) recalls
previous sends; ↓ on the last line walks forward; stepping past the newest
entry restores the in-progress draft. Backed by `loadUserMsgs(sessionId)` plus
in-session sends; no new storage. ↑ with no history is a no-op.

**Status line** (`ContextDashboard.tsx`) — a one-line dim mono bar below the
prompt box: `12.4k / 196k ▂▂▂░░░░░░░ 6% · qwen3.6 · turn 3/40`. The gauge is a
10-cell block meter (`▂` filled / `░` empty) that turns `--cli-err` at ≥80%,
matching today's threshold. Clicking toggles the existing expanded detail
(model/temp, tool + artifact counts, skills, `StatsPanel`), restyled dim mono.
Existing aria labels kept. No-usage state renders `— / —` as today.

**Approval prompt** (`ApprovalPrompt.tsx`) — a `--cli-border` box titled with
the summary, command in coral `pre`, and numbered plain-text options with
hover highlight instead of filled buttons:

```
1. Yes            2. Yes, don't ask again            3. No
```

Keys 1/2/3 map to `approve` / `approve_always` / `deny` while the prompt is
mounted; clicking works too. The key listener attaches only while an approval
is pending and ignores keystrokes when the composer has focus — typing digits
into the prompt box never answers the approval.

## Files touched

| file | change |
|------|--------|
| `web/package.json` | add `@fontsource-variable/jetbrains-mono` |
| `src/main.tsx` | import the mono font |
| `src/index.css` | `--cli-*` tokens per theme, `--font-cli`, `.cli` scoping class |
| `src/components/AgentColumn.tsx` | root gets `cli` class; drop `AgentHeader`; derive busy state |
| `src/components/AgentHeader.tsx` | becomes `SessionBanner` (transcript-top box) |
| `src/components/MessageList.tsx` | user/context restyle; `SessionBanner` first; busy line last |
| `src/components/AnimatedToolCall.tsx` | chip → ⏺/⎿ group with expand + `view →` |
| `src/components/AnimatedReasoningMessage.tsx` | ✻ Thinking restyle |
| `src/components/AnimatedAssistantMessage.tsx`, `AssistantMessage.tsx` | ⏺ prefix + hanging indent |
| `src/components/AnimatedError.tsx` | red ⏺ line |
| `src/components/Composer.tsx` | prompt box, `>` glyph, auto-grow, history |
| `src/components/ContextDashboard.tsx` | status line + block meter; detail kept |
| `src/components/ApprovalPrompt.tsx` | numbered options + 1/2/3 keys |

Untouched: `App.tsx` layout (except any trivial border tweak), TopBar, right
pane (Workspace/Context Explorer), Inspector, all Rust crates, wire protocol.

## Data flow

Everything renders from existing `AnimatedItem` fields (`args`, `content`,
`display`, `resultStatus`, `durationMs`, `parentId`, `streaming`). New pure
helpers — `argSummary(args)`, `resultSummary(content)`, block-meter
formatting — live beside the components for unit testing.

## Testing

- Update `AnimatedToolCall.test.tsx` and `AgentColumn.test.tsx` for new markup.
- New unit tests: `argSummary` / `resultSummary` truncation and edge cases
  (empty, non-string args, multi-line content); composer history navigation
  (↑/↓, draft restore, empty history); status-line/block-meter formatting
  incl. ≥80% state; approval 1/2/3 key mapping incl. composer-focus guard.
- Gate: `npm test` and `npm run typecheck` in `web/` (also covered by
  `bash scripts/ci.sh`).

## Out of scope

- Esc-to-interrupt (needs a wire cancel message + agent-server + agent-core
  work; candidate for its own spec).
- Slash commands, autocomplete, or any composer intelligence.
- Right-pane or Inspector restyling.
- The narrow-layout overlay logic in `App.tsx` (unchanged; the restyled panel
  renders inside it as-is).
