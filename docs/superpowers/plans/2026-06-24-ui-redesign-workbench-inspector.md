# Workbench UI Redesign + Render-Anything Inspector — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the browser UI into a three-pane "Workbench" with an Ink & Sage light/dark design system, and add a full-stack artifact channel so the agent can render arbitrary artifacts (markdown, code, html, mermaid, table, image) into a right-hand Inspector.

**Architecture:** Backend extends the existing `Display` enum (`agent-tools`) with rich variants and adds a builtin `render` tool whose output rides the existing `tool_result` event path — no new wire event, no Worker change. Frontend introduces a token-driven theme, an `AppShell` (ActivityRail · Conversation · Inspector), and one renderer per artifact kind. `TimelineView` is retired into the ActivityRail.

**Tech Stack:** Rust (`agent-tools`, `agent-server`, `agent-runtime-config`; `async-trait`, `serde`, `tokio`, `tempfile`), React 19 + TypeScript + Vite + Tailwind v4, `react-markdown`/`rehype-highlight`, `framer-motion`, `mermaid` (new), vitest + @testing-library/react.

## Global Constraints

- **Wire envelope unchanged.** Keep `{ v, session_id, id?, kind, ... }` and event `payload: { type, ... }`. Only the `Display` leaf enum gains variants. `PROTOCOL_VERSION` stays `1` (additive/backward-compatible).
- **Worker (`cloud/`) is untouched.** It is a transparent relay (verified in `cloud/src/session.ts`).
- **`Display` is an externally-tagged serde enum** → JSON is `{"VariantName": <payload>}`. The TypeScript union must mirror that exact shape.
- **Cargo is not on PATH by default** in this repo — run `source ~/.cargo/env` before any `cargo` command.
- **Backward compatibility:** existing `Display::Text`/`Diff`/`Terminal` variants and their JSON keep working byte-for-byte; older clients ignore unknown `display` variants and fall back to `content`.
- **Tokens only in components** — no raw `zinc-*`/hardcoded hex in components after Task 6; all color comes from CSS variables defined in `index.css`.
- **TDD, frequent commits.** Rust: `cargo test -p <crate>`. Frontend: `npm test` (from `web/`, runs `vitest run`).

---

## Phase A — Backend artifact channel (Rust)

### Task 1: Extend the `Display` enum with rich artifact variants

**Files:**
- Modify: `agent/crates/agent-tools/src/types.rs:35-40` (the `Display` enum) and its `#[cfg(test)] mod tests`
- Test: same file (`agent/crates/agent-tools/src/types.rs` tests module)

**Interfaces:**
- Consumes: nothing new.
- Produces: the extended `Display` enum. New variants (each externally-tagged):
  - `Markdown { text: String, title: Option<String>, id: Option<String> }`
  - `Code { lang: String, filename: Option<String>, text: String, title: Option<String>, id: Option<String> }`
  - `Html { html: String, title: Option<String>, id: Option<String> }`
  - `Mermaid { source: String, title: Option<String>, id: Option<String> }`
  - `Table { columns: Vec<String>, rows: Vec<Vec<String>>, title: Option<String>, id: Option<String> }`
  - `Image { mime: String, data: String, title: Option<String>, id: Option<String> }`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `agent/crates/agent-tools/src/types.rs`:

```rust
    #[test]
    fn display_markdown_round_trips_externally_tagged() {
        let d = Display::Markdown { text: "# Hi".into(), title: Some("Notes".into()), id: None };
        let j = serde_json::to_string(&d).unwrap();
        assert!(j.starts_with("{\"Markdown\":"), "got {j}");
        assert!(j.contains("\"text\":\"# Hi\""));
        let back: Display = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Display::Markdown { .. }));
    }

    #[test]
    fn display_code_carries_lang_and_optional_filename() {
        let d = Display::Code { lang: "rust".into(), filename: Some("a.rs".into()),
            text: "fn x(){}".into(), title: None, id: Some("art-1".into()) };
        let j = serde_json::to_string(&d).unwrap();
        let back: Display = serde_json::from_str(&j).unwrap();
        match back {
            Display::Code { lang, filename, id, .. } => {
                assert_eq!(lang, "rust");
                assert_eq!(filename.as_deref(), Some("a.rs"));
                assert_eq!(id.as_deref(), Some("art-1"));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn display_table_round_trips() {
        let d = Display::Table { columns: vec!["a".into(), "b".into()],
            rows: vec![vec!["1".into(), "2".into()]], title: None, id: None };
        let j = serde_json::to_string(&d).unwrap();
        let back: Display = serde_json::from_str(&j).unwrap();
        assert!(matches!(back, Display::Table { .. }));
    }

    #[test]
    fn existing_diff_variant_json_is_unchanged() {
        let d = Display::Diff { path: "a".into(), before: "x".into(), after: "y".into() };
        let j = serde_json::to_string(&d).unwrap();
        assert_eq!(j, r#"{"Diff":{"path":"a","before":"x","after":"y"}}"#);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source ~/.cargo/env && cargo test -p agent-tools display_ 2>&1 | tail -20`
Expected: FAIL — `no variant named Markdown/Code/Table on Display`.

- [ ] **Step 3: Extend the enum**

Replace the `Display` enum in `agent/crates/agent-tools/src/types.rs` (currently lines 35-40) with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Display {
    Text(String),
    Diff { path: String, before: String, after: String },
    Terminal { command: String, stdout: String, stderr: String, exit_code: i32 },
    Markdown {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
    Code {
        lang: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] filename: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
    Html {
        html: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
    Mermaid {
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
    Image {
        mime: String,
        data: String,
        #[serde(default, skip_serializing_if = "Option::is_none")] title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] id: Option<String>,
    },
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-tools display_ 2>&1 | tail -20`
Expected: PASS (4 tests). Also run `cargo test -p agent-tools 2>&1 | tail -5` to confirm no regression.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-tools/src/types.rs
git commit -m "feat(agent-tools): extend Display enum with rich artifact variants"
```

---

### Task 2: Builtin `render` tool

**Files:**
- Create: `agent/crates/agent-tools/src/render.rs`
- Modify: `agent/crates/agent-tools/src/lib.rs:1-12` (add `mod render;` + re-export)
- Test: `agent/crates/agent-tools/src/render.rs` (inline `tests` module)

**Interfaces:**
- Consumes: `Display` variants from Task 1; `Tool`, `ToolCtx`, `ToolError`, `ToolIntent`, `ToolOutput`, `ToolSchema`, `Access`.
- Produces: `pub struct RenderArtifact;` implementing `Tool` with `name() == "render"`. Args schema:
  `{ kind: "markdown"|"code"|"html"|"mermaid"|"table"|"image" (required), title?, id?, content?, lang?, filename?, mime?, columns?: string[], rows?: string[][] }`.
  Returns `ToolOutput { content: "rendered <kind>[: <title>]", display: Some(<matching Display variant>) }`.

- [ ] **Step 1: Write the failing test**

Create `agent/crates/agent-tools/src/render.rs` with only this tests module first (the rest comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::RenderArtifact;
    use crate::*;
    use serde_json::json;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    fn ctx() -> ToolCtx {
        use std::sync::Arc;
        ToolCtx { workspace: std::env::temp_dir(), timeout: Duration::from_secs(5),
            cancel: CancellationToken::new(), sandbox: Arc::new(crate::HostExecutor) }
    }

    #[tokio::test]
    async fn render_markdown_emits_markdown_display() {
        let out = RenderArtifact.execute(
            json!({"kind":"markdown","title":"Plan","content":"# Hello"}), &ctx())
            .await.unwrap();
        match out.display {
            Some(Display::Markdown { text, title, .. }) => {
                assert_eq!(text, "# Hello");
                assert_eq!(title.as_deref(), Some("Plan"));
            }
            other => panic!("expected Markdown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_code_carries_lang_and_filename() {
        let out = RenderArtifact.execute(
            json!({"kind":"code","lang":"rust","filename":"a.rs","content":"fn x(){}"}), &ctx())
            .await.unwrap();
        assert!(matches!(out.display, Some(Display::Code { .. })));
    }

    #[tokio::test]
    async fn render_table_uses_columns_and_rows() {
        let out = RenderArtifact.execute(
            json!({"kind":"table","columns":["a","b"],"rows":[["1","2"]]}), &ctx())
            .await.unwrap();
        match out.display {
            Some(Display::Table { columns, rows, .. }) => {
                assert_eq!(columns, vec!["a", "b"]);
                assert_eq!(rows, vec![vec!["1", "2"]]);
            }
            other => panic!("expected Table, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn render_rejects_unknown_kind() {
        let err = RenderArtifact.execute(json!({"kind":"wat","content":"x"}), &ctx())
            .await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs(_)));
    }

    #[test]
    fn render_intent_is_read() {
        let i = RenderArtifact.intent(&json!({"kind":"markdown","content":"x"})).unwrap();
        assert_eq!(i.access, Access::Read);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source ~/.cargo/env && cargo test -p agent-tools render_ 2>&1 | tail -20`
Expected: FAIL — `cannot find type RenderArtifact` / unresolved `super::RenderArtifact`.

- [ ] **Step 3: Implement the tool**

Prepend to `agent/crates/agent-tools/src/render.rs` (above the tests module):

```rust
use crate::{Access, Display, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::json;

fn str_arg(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string field `{key}`")))
}
fn opt_str(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(str::to_string)
}

/// Builtin tool: render an arbitrary artifact into the browser Inspector.
/// Side-effect-free; produces a `Display` payload on the existing tool_result path.
pub struct RenderArtifact;

#[async_trait]
impl Tool for RenderArtifact {
    fn name(&self) -> &str { "render" }
    fn description(&self) -> &str {
        "Render an artifact (markdown, code, html, mermaid diagram, table, or image) into the user's Inspector panel."
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name().into(),
            description: self.description().into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "kind": {"type": "string",
                        "enum": ["markdown","code","html","mermaid","table","image"]},
                    "title": {"type": "string"},
                    "id": {"type": "string", "description": "stable id; re-rendering the same id replaces the artifact"},
                    "content": {"type": "string",
                        "description": "primary payload: markdown/html/mermaid source, code text, or base64 image data"},
                    "lang": {"type": "string", "description": "code language (kind=code)"},
                    "filename": {"type": "string", "description": "code filename (kind=code)"},
                    "mime": {"type": "string", "description": "image mime type (kind=image)"},
                    "columns": {"type": "array", "items": {"type": "string"}},
                    "rows": {"type": "array", "items": {"type": "array", "items": {"type": "string"}}}
                },
                "required": ["kind"]
            }),
        }
    }
    fn intent(&self, args: &serde_json::Value) -> Result<ToolIntent, ToolError> {
        let kind = str_arg(args, "kind")?;
        Ok(ToolIntent { tool: "render".into(), access: Access::Read, paths: vec![],
            command: None, summary: format!("render {kind}") })
    }
    async fn execute(&self, args: serde_json::Value, _ctx: &ToolCtx)
        -> Result<ToolOutput, ToolError> {
        let kind = str_arg(&args, "kind")?;
        let title = opt_str(&args, "title");
        let id = opt_str(&args, "id");
        let display = match kind.as_str() {
            "markdown" => Display::Markdown { text: str_arg(&args, "content")?, title: title.clone(), id },
            "html" => Display::Html { html: str_arg(&args, "content")?, title: title.clone(), id },
            "mermaid" => Display::Mermaid { source: str_arg(&args, "content")?, title: title.clone(), id },
            "code" => Display::Code {
                lang: opt_str(&args, "lang").unwrap_or_else(|| "text".into()),
                filename: opt_str(&args, "filename"),
                text: str_arg(&args, "content")?, title: title.clone(), id },
            "image" => Display::Image {
                mime: opt_str(&args, "mime").unwrap_or_else(|| "image/png".into()),
                data: str_arg(&args, "content")?, title: title.clone(), id },
            "table" => {
                let columns: Vec<String> = serde_json::from_value(
                    args.get("columns").cloned().unwrap_or(json!([])))
                    .map_err(|e| ToolError::InvalidArgs(format!("columns: {e}")))?;
                let rows: Vec<Vec<String>> = serde_json::from_value(
                    args.get("rows").cloned().unwrap_or(json!([])))
                    .map_err(|e| ToolError::InvalidArgs(format!("rows: {e}")))?;
                Display::Table { columns, rows, title: title.clone(), id }
            }
            other => return Err(ToolError::InvalidArgs(format!("unknown kind `{other}`"))),
        };
        let ack = match &title { Some(t) => format!("rendered {kind}: {t}"), None => format!("rendered {kind}") };
        Ok(ToolOutput { content: ack, display: Some(display) })
    }
}
```

- [ ] **Step 4: Wire the module into the crate**

In `agent/crates/agent-tools/src/lib.rs`, add `mod render;` after `mod registry;` and `pub use render::*;` after `pub use registry::*;`. Result:

```rust
//! Shared tool vocabulary and the `Tool` trait.
mod types;
mod tool;
mod registry;
mod render;
pub mod fs;
pub mod shell;
pub mod git;
pub mod sandbox;
pub use types::*;
pub use tool::*;
pub use registry::*;
pub use render::*;
pub use sandbox::*;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-tools render_ 2>&1 | tail -20`
Expected: PASS (5 tests).

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-tools/src/render.rs agent/crates/agent-tools/src/lib.rs
git commit -m "feat(agent-tools): add builtin render tool for Inspector artifacts"
```

---

### Task 3: Register `render` in the default tool registry + lock the wire contract

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/lib.rs:67-78` (`build_registry`) and its imports
- Test: `agent/crates/agent-runtime-config/src/lib.rs` (inline tests) and `agent/crates/agent-server/src/wire.rs` (inline tests)

**Interfaces:**
- Consumes: `RenderArtifact` from Task 2.
- Produces: `render` present in the registry returned by `build_registry`; an agent-server round-trip test proving a `tool_result` carrying `Display::Markdown` serializes under `kind:"event"` / `type:"tool_result"`.

- [ ] **Step 1: Write the failing tests**

Add to the tests module in `agent/crates/agent-runtime-config/src/lib.rs`:

```rust
    #[test]
    fn build_registry_includes_render() {
        let r = build_registry(&[]);
        assert!(r.get("render").is_some(), "render tool must be registered");
    }
```

Add to the tests module in `agent/crates/agent-server/src/wire.rs`:

```rust
    #[test]
    fn tool_result_with_markdown_display_round_trips() {
        use agent_tools::Display;
        let payload = WireEvent::ToolResult {
            name: "render".into(),
            content: "rendered markdown".into(),
            display: Some(Display::Markdown { text: "# Hi".into(), title: Some("Plan".into()), id: None }),
        };
        let env = WireEnvelope { v: PROTOCOL_VERSION, session_id: "s".into(), id: None,
            body: WireBody::Event { payload } };
        let j = serde_json::to_string(&env).unwrap();
        assert!(j.contains("\"kind\":\"event\""));
        assert!(j.contains("\"type\":\"tool_result\""));
        assert!(j.contains("\"Markdown\""));
        let back: WireEnvelope = serde_json::from_str(&j).unwrap();
        assert!(matches!(back.body, WireBody::Event { .. }));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `source ~/.cargo/env && cargo test -p agent-runtime-config build_registry_includes_render 2>&1 | tail -15`
Expected: FAIL — `render` not registered.
(The agent-server test will compile but assert-fail only if the import is missing; run it too: `cargo test -p agent-server tool_result_with_markdown 2>&1 | tail -15`.)

- [ ] **Step 3: Register the tool**

In `agent/crates/agent-runtime-config/src/lib.rs`, find the `use agent_tools::{...}` import that brings in `ReadFile, WriteFile, ...` and add `RenderArtifact` to it. Then in `build_registry`, add the registration line after the fs/shell/git tools:

```rust
    r.register(Arc::new(RenderArtifact));
```

(Place it right before `r.register(Arc::new(FetchUrl::new(...)));` so builtins stay grouped.)

- [ ] **Step 4: Run the tests to verify they pass**

Run: `source ~/.cargo/env && cargo test -p agent-runtime-config build_registry_includes_render && cargo test -p agent-server tool_result_with_markdown 2>&1 | tail -15`
Expected: PASS both.

- [ ] **Step 5: Full backend regression check**

Run: `source ~/.cargo/env && cargo test -p agent-tools -p agent-runtime-config -p agent-server 2>&1 | tail -15`
Expected: all green.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-runtime-config/src/lib.rs agent/crates/agent-server/src/wire.rs
git commit -m "feat: register render tool and lock artifact wire contract"
```

---

## Phase B — Frontend: types, design system, shell

### Task 4: Mirror the new `Display` variants in TypeScript

**Files:**
- Modify: `web/src/wire.ts:3-6` (the `Display` union)
- Test: `web/test/wire-display.test.ts` (create)

**Interfaces:**
- Consumes: the Rust JSON shapes from Task 1.
- Produces: the extended TS `Display` union (exact mirror):

```ts
export type Display =
  | { Text: string }
  | { Diff: { path: string; before: string; after: string } }
  | { Terminal: { command: string; stdout: string; stderr: string; exit_code: number } }
  | { Markdown: { text: string; title?: string; id?: string } }
  | { Code: { lang: string; filename?: string; text: string; title?: string; id?: string } }
  | { Html: { html: string; title?: string; id?: string } }
  | { Mermaid: { source: string; title?: string; id?: string } }
  | { Table: { columns: string[]; rows: string[][]; title?: string; id?: string } }
  | { Image: { mime: string; data: string; title?: string; id?: string } };
```

- [ ] **Step 1: Write the failing test**

Create `web/test/wire-display.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { parseInbound } from "../src/wire";

describe("Display variants over the wire", () => {
  it("parses a tool_result carrying a Markdown artifact", () => {
    const raw = JSON.stringify({
      v: 1, session_id: "s", kind: "event",
      payload: { type: "tool_result", name: "render", content: "rendered markdown",
        display: { Markdown: { text: "# Hi", title: "Plan" } } },
    });
    const msg = parseInbound(raw);
    expect(msg?.kind).toBe("event");
    if (msg?.kind === "event" && msg.payload.type === "tool_result") {
      expect(msg.payload.display).toEqual({ Markdown: { text: "# Hi", title: "Plan" } });
    } else {
      throw new Error("expected a tool_result event");
    }
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- wire-display 2>&1 | tail -20`
Expected: FAIL — type error / `display` typed without `Markdown` (the `.toEqual` will pass at runtime, but `tsc` in the typecheck step would reject; the failing signal here is the missing variant). If it passes at runtime, still proceed — the union must be updated for downstream tasks.

- [ ] **Step 3: Update the union**

Replace `web/src/wire.ts` lines 3-6 with the union shown in **Produces** above.

- [ ] **Step 4: Run the test + typecheck to verify they pass**

Run (from `web/`): `npm test -- wire-display 2>&1 | tail -10 && npm run typecheck 2>&1 | tail -10`
Expected: test PASS, typecheck clean.

- [ ] **Step 5: Commit**

```bash
git add web/src/wire.ts web/test/wire-display.test.ts
git commit -m "feat(web): mirror rich Display artifact variants in wire types"
```

---

### Task 5: Theme tokens + light/dark switch

**Files:**
- Modify: `web/src/index.css` (replace contents)
- Modify: `web/src/storage.ts` (add theme persistence)
- Create: `web/src/theme.ts`
- Create: `web/src/components/ThemeToggle.tsx`
- Test: `web/test/theme.test.ts` (create)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `theme.ts`: `export type Theme = "light" | "dark";` `export function resolveInitialTheme(stored: Theme | null, prefersDark: boolean): Theme;` `export function applyTheme(theme: Theme): void;` (sets `document.documentElement.dataset.theme`).
  - `storage.ts`: `export function loadTheme(): Theme | null;` `export function saveTheme(t: Theme): void;`
  - `ThemeToggle`: `export function ThemeToggle({ theme, onToggle }: { theme: Theme; onToggle: () => void })`.
  - CSS custom properties (consumed by all later tasks): `--surface-base`, `--surface-raised`, `--surface-overlay`, `--border`, `--text-strong`, `--text`, `--text-muted`, `--accent`, `--accent-fg`, `--accent-2`, `--state-run`, `--state-done`, `--state-error`, `--ring`, defined under `:root[data-theme="light"]` and `:root[data-theme="dark"]`.

- [ ] **Step 1: Write the failing test**

Create `web/test/theme.test.ts`:

```ts
import { describe, it, expect } from "vitest";
import { resolveInitialTheme } from "../src/theme";

describe("resolveInitialTheme", () => {
  it("prefers a stored choice", () => {
    expect(resolveInitialTheme("light", true)).toBe("light");
    expect(resolveInitialTheme("dark", false)).toBe("dark");
  });
  it("falls back to system preference", () => {
    expect(resolveInitialTheme(null, true)).toBe("dark");
    expect(resolveInitialTheme(null, false)).toBe("light");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- theme 2>&1 | tail -10`
Expected: FAIL — cannot find module `../src/theme`.

- [ ] **Step 3: Create `theme.ts`**

```ts
export type Theme = "light" | "dark";

export function resolveInitialTheme(stored: Theme | null, prefersDark: boolean): Theme {
  if (stored === "light" || stored === "dark") return stored;
  return prefersDark ? "dark" : "light";
}

export function applyTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
}
```

- [ ] **Step 4: Add theme persistence to `storage.ts`**

Append to `web/src/storage.ts`:

```ts
import type { Theme } from "./theme";

const THEME_KEY = "agent.theme";

export function loadTheme(): Theme | null {
  const v = localStorage.getItem(THEME_KEY);
  return v === "light" || v === "dark" ? v : null;
}

export function saveTheme(t: Theme): void {
  try { localStorage.setItem(THEME_KEY, t); } catch { /* ignore */ }
}
```

- [ ] **Step 5: Replace `index.css` with the token system**

Replace the entire contents of `web/src/index.css`:

```css
@import "tailwindcss";

:root[data-theme="light"] {
  --surface-base: #fcfcfa;
  --surface-raised: #f5f6f1;
  --surface-overlay: #ffffff;
  --border: #e4e6e0;
  --text-strong: #16181a;
  --text: #3f4448;
  --text-muted: #8a9082;
  --accent: #4f7a52;
  --accent-fg: #ffffff;
  --accent-2: #c4622d;
  --state-run: #c4622d;
  --state-done: #4f7a52;
  --state-error: #b3402e;
  --ring: #4f7a52;
}

:root[data-theme="dark"] {
  --surface-base: #16181a;
  --surface-raised: #1d201e;
  --surface-overlay: #22271f;
  --border: #2a2e2b;
  --text-strong: #f3f4ee;
  --text: #c4cabe;
  --text-muted: #6a6f63;
  --accent: #6fae72;
  --accent-fg: #16181a;
  --accent-2: #d9824f;
  --state-run: #d9824f;
  --state-done: #6fae72;
  --state-error: #e0654f;
  --ring: #6fae72;
}

/* default before JS applies a theme: match light */
:root { color-scheme: light dark; }

html, body, #root { height: 100%; }
body {
  margin: 0;
  background: var(--surface-base);
  color: var(--text);
  font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
}

@media (prefers-reduced-motion: reduce) {
  * { animation-duration: 0.01ms !important; transition-duration: 0.01ms !important; }
}
```

- [ ] **Step 6: Create `ThemeToggle.tsx`**

```tsx
import type { Theme } from "../theme";

export function ThemeToggle({ theme, onToggle }: { theme: Theme; onToggle: () => void }) {
  return (
    <button
      onClick={onToggle}
      aria-label="toggle theme"
      title={theme === "dark" ? "Switch to light" : "Switch to dark"}
      style={{ color: "var(--text-muted)" }}
      className="hover:opacity-80"
    >
      {theme === "dark" ? "◐" : "◑"}
    </button>
  );
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run (from `web/`): `npm test -- theme 2>&1 | tail -10`
Expected: PASS (2 tests).

- [ ] **Step 8: Commit**

```bash
git add web/src/index.css web/src/theme.ts web/src/storage.ts web/src/components/ThemeToggle.tsx web/test/theme.test.ts
git commit -m "feat(web): add Ink & Sage token system + light/dark theme switch"
```

---

### Task 6: Wire theme into `App` + restyle `StatusBar`

**Files:**
- Modify: `web/src/App.tsx`
- Modify: `web/src/components/StatusBar.tsx`
- Test: `web/test/shell-components.test.tsx` (extend with a StatusBar theme-toggle test)

**Interfaces:**
- Consumes: `resolveInitialTheme`, `applyTheme`, `Theme` (Task 5); `loadTheme`, `saveTheme` (Task 5); `ThemeToggle` (Task 5).
- Produces: `App` owns `theme` state + a `toggleTheme` callback; `StatusBar` gains props `theme: Theme` and `onToggleTheme: () => void` and renders `<ThemeToggle>`.

- [ ] **Step 1: Write the failing test**

Add to `web/test/shell-components.test.tsx`:

```tsx
import { StatusBar } from "../src/components/StatusBar";
// (existing imports stay)

it("StatusBar renders a theme toggle and fires onToggleTheme", async () => {
  const onToggleTheme = vi.fn();
  render(<StatusBar online status="open" onSignOut={() => {}}
    theme="dark" onToggleTheme={onToggleTheme} />);
  const btn = screen.getByLabelText("toggle theme");
  btn.click();
  expect(onToggleTheme).toHaveBeenCalledOnce();
});
```

Ensure `vi` is imported at the top of the file (`import { describe, it, expect, vi } from "vitest";`).

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- shell-components 2>&1 | tail -20`
Expected: FAIL — `StatusBar` has no `theme`/`onToggleTheme` props; no toggle present.

- [ ] **Step 3: Restyle `StatusBar` + add the toggle**

Replace `web/src/components/StatusBar.tsx`:

```tsx
import type { ConnectionStatus } from "../state";
import type { Theme } from "../theme";
import { ThemeToggle } from "./ThemeToggle";

export function StatusBar({ online, status, onSignOut, onOpenSettings, settingsDisabled, theme, onToggleTheme }:
  { online: boolean; status: ConnectionStatus; onSignOut: () => void;
    onOpenSettings?: () => void; settingsDisabled?: boolean;
    theme: Theme; onToggleTheme: () => void }) {
  return (
    <div className="flex items-center justify-between px-4 py-2 text-sm"
      style={{ background: "var(--surface-raised)", borderBottom: "1px solid var(--border)" }}>
      <div className="flex items-center gap-2">
        <span className="h-2 w-2 rounded-full"
          style={{ background: online ? "var(--state-done)" : "var(--text-muted)" }} />
        <span style={{ color: "var(--text)" }}>{online ? "agent online" : "agent offline"}</span>
        <span style={{ color: "var(--text-muted)" }}>· {status}</span>
      </div>
      <div className="flex items-center gap-3">
        <ThemeToggle theme={theme} onToggle={onToggleTheme} />
        {onOpenSettings && (
          <button onClick={onOpenSettings} disabled={settingsDisabled}
            className="disabled:opacity-40 disabled:cursor-not-allowed hover:opacity-80"
            style={{ color: "var(--text-muted)" }} aria-label="settings">⚙</button>
        )}
        <button onClick={onSignOut} className="hover:opacity-80"
          style={{ color: "var(--text-muted)" }}>sign out</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Wire theme state into `App.tsx`**

In `web/src/App.tsx`: add imports
```tsx
import { resolveInitialTheme, applyTheme, type Theme } from "./theme";
import { loadTheme, saveTheme } from "./storage";
```
(merge the `storage` import with the existing one). Inside `App`, after the other `useState` calls, add:
```tsx
  const [theme, setTheme] = useState<Theme>(() =>
    resolveInitialTheme(loadTheme(), window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false));
  useEffect(() => { applyTheme(theme); }, [theme]);
  const toggleTheme = () => setTheme((t) => { const next = t === "dark" ? "light" : "dark"; saveTheme(next); return next; });
```
Then pass the props to `<StatusBar ... theme={theme} onToggleTheme={toggleTheme} />`. Also change the pairing wrapper `<div className="h-screen bg-zinc-950">` to `<div className="h-screen" style={{ background: "var(--surface-base)" }}>` and the main wrapper `<div className="flex h-screen flex-col bg-zinc-950">` to `<div className="flex h-screen flex-col" style={{ background: "var(--surface-base)" }}>`.

Note: `applyTheme` runs in `useEffect`; also call `applyTheme(theme)` once eagerly is unnecessary (effect runs on mount).

- [ ] **Step 5: Run the test to verify it passes**

Run (from `web/`): `npm test -- shell-components 2>&1 | tail -15 && npm run typecheck 2>&1 | tail -10`
Expected: PASS + clean typecheck.

- [ ] **Step 6: Commit**

```bash
git add web/src/App.tsx web/src/components/StatusBar.tsx web/test/shell-components.test.tsx
git commit -m "feat(web): wire theme state into App and restyle StatusBar with tokens"
```

---

### Task 7: Artifact model helper (`artifactsFrom`)

**Files:**
- Modify: `web/src/state.ts` (add `InspectorArtifact` + `artifactsFrom`)
- Test: `web/test/artifacts.test.ts` (create)

**Interfaces:**
- Consumes: `Item`, `Display` (existing in `state.ts`/`wire.ts`).
- Produces:
  ```ts
  export interface InspectorArtifact { key: string; title: string; display: Display; }
  export function artifactsFrom(items: Item[]): InspectorArtifact[];
  ```
  Returns one entry per tool `Item` that has a `display`, in order. `key` = `"art-" + index`. `title` = the display's `title` field if present, else the tool `name`.

- [ ] **Step 1: Write the failing test**

Create `web/test/artifacts.test.ts`:

```ts
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
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- artifacts 2>&1 | tail -10`
Expected: FAIL — `artifactsFrom` not exported.

- [ ] **Step 3: Implement the helper**

Add to `web/src/state.ts` (near the bottom, after `turnGroupsFrom`):

```ts
export interface InspectorArtifact { key: string; title: string; display: Display; }

/** One Inspector artifact per tool Item that carries a display, in order. */
export function artifactsFrom(items: Item[]): InspectorArtifact[] {
  const out: InspectorArtifact[] = [];
  items.forEach((it, i) => {
    if (it.kind === "tool" && it.display) {
      const title = displayTitle(it.display) ?? it.name;
      out.push({ key: `art-${i}`, title, display: it.display });
    }
  });
  return out;
}

function displayTitle(d: Display): string | undefined {
  // every rich variant carries an optional title; older variants don't.
  const v = Object.values(d)[0] as { title?: string };
  return v && typeof v === "object" ? v.title : undefined;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `web/`): `npm test -- artifacts 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add web/src/state.ts web/test/artifacts.test.ts
git commit -m "feat(web): add artifactsFrom helper deriving Inspector artifacts"
```

---

## Phase C — Frontend: Inspector + renderers

### Task 8: `ArtifactRenderer` — markdown, code, diff, terminal, table, image

**Files:**
- Create: `web/src/components/inspector/ArtifactRenderer.tsx`
- Modify: `web/src/components/MarkdownText.tsx` (export-only; reused as-is)
- Test: `web/test/artifact-renderer.test.tsx` (create)

**Interfaces:**
- Consumes: `Display` (wire.ts); existing `DiffView`, `TerminalBlock`, `MarkdownText`.
- Produces: `export function ArtifactRenderer({ display }: { display: Display })`. Renders by variant. (Html + Mermaid added in Task 9.)

- [ ] **Step 1: Write the failing test**

Create `web/test/artifact-renderer.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ArtifactRenderer } from "../src/components/inspector/ArtifactRenderer";

describe("ArtifactRenderer", () => {
  it("renders a Markdown artifact", () => {
    render(<ArtifactRenderer display={{ Markdown: { text: "# Title here" } }} />);
    expect(screen.getByText("Title here")).toBeInTheDocument();
  });
  it("renders a Code artifact with filename header", () => {
    render(<ArtifactRenderer display={{ Code: { lang: "rust", filename: "a.rs", text: "fn x(){}" } }} />);
    expect(screen.getByText("a.rs")).toBeInTheDocument();
    expect(screen.getByText(/fn x/)).toBeInTheDocument();
  });
  it("renders a Table artifact", () => {
    render(<ArtifactRenderer display={{ Table: { columns: ["A", "B"], rows: [["1", "2"]] } }} />);
    expect(screen.getByText("A")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });
  it("renders a Diff artifact", () => {
    render(<ArtifactRenderer display={{ Diff: { path: "a.txt", before: "foo\n", after: "bar\n" } }} />);
    expect(screen.getByText("a.txt")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- artifact-renderer 2>&1 | tail -15`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `ArtifactRenderer`**

Create `web/src/components/inspector/ArtifactRenderer.tsx`:

```tsx
import type { Display } from "../../wire";
import { DiffView } from "../DiffView";
import { TerminalBlock } from "../TerminalBlock";
import { MarkdownText } from "../MarkdownText";

export function ArtifactRenderer({ display }: { display: Display }) {
  if ("Text" in display) {
    return <pre className="whitespace-pre-wrap p-3 font-mono text-sm" style={{ color: "var(--text)" }}>{display.Text}</pre>;
  }
  if ("Markdown" in display) {
    return <div className="p-3"><MarkdownText text={display.Markdown.text} /></div>;
  }
  if ("Code" in display) {
    const { filename, lang, text } = display.Code;
    return (
      <div className="m-3 rounded" style={{ border: "1px solid var(--border)" }}>
        <div className="px-2 py-1 font-mono text-xs"
          style={{ background: "var(--surface-raised)", color: "var(--text-muted)", borderBottom: "1px solid var(--border)" }}>
          {filename ?? lang}
        </div>
        <MarkdownText text={"```" + lang + "\n" + text + "\n```"} />
      </div>
    );
  }
  if ("Diff" in display) {
    return <div className="p-3"><DiffView path={display.Diff.path} before={display.Diff.before} after={display.Diff.after} /></div>;
  }
  if ("Terminal" in display) {
    const t = display.Terminal;
    return <div className="p-3"><TerminalBlock command={t.command} stdout={t.stdout} stderr={t.stderr} exitCode={t.exit_code} /></div>;
  }
  if ("Table" in display) {
    const { columns, rows } = display.Table;
    return (
      <div className="p-3 overflow-x-auto">
        <table className="w-full text-sm" style={{ color: "var(--text)" }}>
          <thead>
            <tr>{columns.map((c, i) => (
              <th key={i} className="px-2 py-1 text-left font-semibold"
                style={{ color: "var(--text-strong)", borderBottom: "1px solid var(--border)" }}>{c}</th>
            ))}</tr>
          </thead>
          <tbody>
            {rows.map((r, ri) => (
              <tr key={ri}>{r.map((cell, ci) => (
                <td key={ci} className="px-2 py-1" style={{ borderBottom: "1px solid var(--border)" }}>{cell}</td>
              ))}</tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }
  if ("Image" in display) {
    const { mime, data } = display.Image;
    const src = data.startsWith("http") || data.startsWith("data:") ? data : `data:${mime};base64,${data}`;
    return <div className="p-3"><img src={src} alt="rendered artifact" className="max-w-full rounded" /></div>;
  }
  // Html and Mermaid are added in Task 9.
  return null;
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `web/`): `npm test -- artifact-renderer 2>&1 | tail -15`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/components/inspector/ArtifactRenderer.tsx web/test/artifact-renderer.test.tsx
git commit -m "feat(web): ArtifactRenderer for markdown/code/diff/terminal/table/image"
```

---

### Task 9: HTML (sandboxed) + Mermaid renderers

**Files:**
- Modify: `web/package.json` (add `mermaid`)
- Create: `web/src/components/inspector/HtmlArtifact.tsx`
- Create: `web/src/components/inspector/MermaidArtifact.tsx`
- Modify: `web/src/components/inspector/ArtifactRenderer.tsx` (route `Html`/`Mermaid`)
- Test: `web/test/artifact-renderer.test.tsx` (extend)

**Interfaces:**
- Consumes: `Display` `Html`/`Mermaid` variants.
- Produces: `HtmlArtifact({ html }: { html: string })` rendering a sandboxed `<iframe>`; `MermaidArtifact({ source }: { source: string })` lazy-rendering an SVG.

- [ ] **Step 1: Add the dependency**

Run (from `web/`): `npm install mermaid@^11`
Expected: `mermaid` added to `dependencies`.

- [ ] **Step 2: Write the failing test**

Add to `web/test/artifact-renderer.test.tsx`:

```tsx
it("renders an Html artifact inside a sandboxed iframe", () => {
  const { container } = render(<ArtifactRenderer display={{ Html: { html: "<p>hello</p>" } }} />);
  const iframe = container.querySelector("iframe");
  expect(iframe).not.toBeNull();
  expect(iframe?.getAttribute("sandbox")).toBe("");
  expect(iframe?.getAttribute("srcdoc")).toContain("<p>hello</p>");
});
it("renders a Mermaid artifact container", () => {
  const { container } = render(<ArtifactRenderer display={{ Mermaid: { source: "graph TD; A-->B;" } }} />);
  expect(container.querySelector("[data-mermaid]")).not.toBeNull();
});
```

- [ ] **Step 3: Run the test to verify it fails**

Run (from `web/`): `npm test -- artifact-renderer 2>&1 | tail -15`
Expected: FAIL — iframe/`[data-mermaid]` not found (Html/Mermaid return null).

- [ ] **Step 4: Implement `HtmlArtifact`**

Create `web/src/components/inspector/HtmlArtifact.tsx`:

```tsx
// Agent HTML is rendered in a fully sandboxed iframe (empty sandbox = no scripts,
// no same-origin) so it cannot touch the app, cookies, or storage.
export function HtmlArtifact({ html }: { html: string }) {
  return (
    <iframe
      title="rendered html"
      sandbox=""
      srcDoc={html}
      className="h-full w-full"
      style={{ border: "none", minHeight: "240px", background: "var(--surface-overlay)" }}
    />
  );
}
```

- [ ] **Step 5: Implement `MermaidArtifact`**

Create `web/src/components/inspector/MermaidArtifact.tsx`:

```tsx
import { useEffect, useRef, useState } from "react";

// Lazy-load mermaid so it stays out of the initial bundle.
export function MermaidArtifact({ source }: { source: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const mermaid = (await import("mermaid")).default;
        mermaid.initialize({ startOnLoad: false, theme: "neutral" });
        const { svg } = await mermaid.render("m" + Math.abs(hash(source)), source);
        if (!cancelled && ref.current) ref.current.innerHTML = svg;
      } catch (e) {
        if (!cancelled) setError(e instanceof Error ? e.message : "diagram error");
      }
    })();
    return () => { cancelled = true; };
  }, [source]);

  if (error) return <pre className="p-3 text-sm" style={{ color: "var(--state-error)" }}>{error}</pre>;
  return <div data-mermaid ref={ref} className="p-3" />;
}

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) { h = (h << 5) - h + s.charCodeAt(i); h |= 0; }
  return h;
}
```

- [ ] **Step 6: Route the two variants in `ArtifactRenderer`**

In `web/src/components/inspector/ArtifactRenderer.tsx`, add imports at the top:
```tsx
import { HtmlArtifact } from "./HtmlArtifact";
import { MermaidArtifact } from "./MermaidArtifact";
```
Replace the `// Html and Mermaid are added in Task 9.` comment with:
```tsx
  if ("Html" in display) {
    return <HtmlArtifact html={display.Html.html} />;
  }
  if ("Mermaid" in display) {
    return <MermaidArtifact source={display.Mermaid.source} />;
  }
```

- [ ] **Step 7: Run the test to verify it passes**

Run (from `web/`): `npm test -- artifact-renderer 2>&1 | tail -15`
Expected: PASS (6 tests). The mermaid `import()` is dynamic; the container renders synchronously so the test passes without awaiting render.

- [ ] **Step 8: Commit**

```bash
git add web/package.json web/package-lock.json web/src/components/inspector/HtmlArtifact.tsx web/src/components/inspector/MermaidArtifact.tsx web/src/components/inspector/ArtifactRenderer.tsx web/test/artifact-renderer.test.tsx
git commit -m "feat(web): sandboxed HTML + lazy Mermaid artifact renderers"
```

---

### Task 10: `Inspector` panel (tabs + active selection)

**Files:**
- Create: `web/src/components/inspector/Inspector.tsx`
- Test: `web/test/inspector.test.tsx` (create)

**Interfaces:**
- Consumes: `InspectorArtifact` (Task 7); `ArtifactRenderer` (Tasks 8-9).
- Produces:
  ```ts
  export function Inspector({ artifacts, activeKey, onSelect, onClose }:
    { artifacts: InspectorArtifact[]; activeKey: string | null;
      onSelect: (key: string) => void; onClose: () => void });
  ```
  Renders a tab per artifact (label = `title`); the active tab (or the last artifact if `activeKey` is null) renders via `ArtifactRenderer`. Empty state when `artifacts` is empty.

- [ ] **Step 1: Write the failing test**

Create `web/test/inspector.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { Inspector } from "../src/components/inspector/Inspector";
import type { InspectorArtifact } from "../src/state";

const arts: InspectorArtifact[] = [
  { key: "art-1", title: "Plan", display: { Markdown: { text: "# Plan body" } } },
  { key: "art-2", title: "token.rs", display: { Code: { lang: "rust", filename: "token.rs", text: "fn x(){}" } } },
];

describe("Inspector", () => {
  it("shows an empty state when there are no artifacts", () => {
    render(<Inspector artifacts={[]} activeKey={null} onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByText(/nothing to inspect/i)).toBeInTheDocument();
  });
  it("renders a tab per artifact and shows the active one", () => {
    render(<Inspector artifacts={arts} activeKey="art-1" onSelect={() => {}} onClose={() => {}} />);
    expect(screen.getByRole("tab", { name: "Plan" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "token.rs" })).toBeInTheDocument();
    expect(screen.getByText("Plan body")).toBeInTheDocument();
  });
  it("fires onSelect when a tab is clicked", () => {
    const onSelect = vi.fn();
    render(<Inspector artifacts={arts} activeKey="art-1" onSelect={onSelect} onClose={() => {}} />);
    screen.getByRole("tab", { name: "token.rs" }).click();
    expect(onSelect).toHaveBeenCalledWith("art-2");
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- inspector 2>&1 | tail -15`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `Inspector`**

Create `web/src/components/inspector/Inspector.tsx`:

```tsx
import type { InspectorArtifact } from "../../state";
import { ArtifactRenderer } from "./ArtifactRenderer";

export function Inspector({ artifacts, activeKey, onSelect, onClose }:
  { artifacts: InspectorArtifact[]; activeKey: string | null;
    onSelect: (key: string) => void; onClose: () => void }) {
  if (artifacts.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6 text-sm"
        style={{ color: "var(--text-muted)" }}>
        Nothing to inspect yet.
      </div>
    );
  }
  const active = artifacts.find((a) => a.key === activeKey) ?? artifacts[artifacts.length - 1];
  return (
    <div className="flex h-full flex-col" style={{ background: "var(--surface-raised)" }}>
      <div className="flex items-center gap-1 px-2 pt-2" role="tablist"
        style={{ borderBottom: "1px solid var(--border)" }}>
        {artifacts.map((a) => {
          const on = a.key === active.key;
          return (
            <button key={a.key} role="tab" aria-selected={on} onClick={() => onSelect(a.key)}
              className="rounded-t px-3 py-1 text-xs"
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
        <button onClick={onClose} aria-label="close inspector"
          className="ml-auto px-2 text-xs hover:opacity-80" style={{ color: "var(--text-muted)" }}>✕</button>
      </div>
      <div className="flex-1 overflow-auto" style={{ background: "var(--surface-overlay)" }}>
        <ArtifactRenderer display={active.display} />
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `web/`): `npm test -- inspector 2>&1 | tail -15`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/components/inspector/Inspector.tsx web/test/inspector.test.tsx
git commit -m "feat(web): Inspector panel with artifact tabs and empty state"
```

---

## Phase D — Frontend: shell assembly + restyle

### Task 11: `ActivityRail` (retires `TimelineView`)

**Files:**
- Create: `web/src/components/ActivityRail.tsx`
- Test: `web/test/activity-rail.test.tsx` (create)

**Interfaces:**
- Consumes: `Item` (state.ts).
- Produces:
  ```ts
  export function ActivityRail({ items, sessionLabel, onOpenSettings, collapsed, onToggleCollapse }:
    { items: Item[]; sessionLabel: string; onOpenSettings?: () => void;
      collapsed: boolean; onToggleCollapse: () => void });
  ```
  Lists tool items with a status dot (running → `--state-run`, done → `--state-done`); shows `sessionLabel`; a settings entry at the bottom; collapses to an icon strip.

- [ ] **Step 1: Write the failing test**

Create `web/test/activity-rail.test.tsx`:

```tsx
import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { ActivityRail } from "../src/components/ActivityRail";
import type { Item } from "../src/state";

const items: Item[] = [
  { kind: "user", text: "hi" },
  { kind: "tool", name: "edit_file", args: {}, status: "done" },
  { kind: "tool", name: "execute_command", args: {}, status: "running" },
];

describe("ActivityRail", () => {
  it("lists tool activity with the session label", () => {
    render(<ActivityRail items={items} sessionLabel="auth-refactor"
      collapsed={false} onToggleCollapse={() => {}} />);
    expect(screen.getByText("auth-refactor")).toBeInTheDocument();
    expect(screen.getByText("edit_file")).toBeInTheDocument();
    expect(screen.getByText("execute_command")).toBeInTheDocument();
  });
  it("hides labels when collapsed", () => {
    render(<ActivityRail items={items} sessionLabel="auth-refactor"
      collapsed={true} onToggleCollapse={() => {}} />);
    expect(screen.queryByText("edit_file")).not.toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run (from `web/`): `npm test -- activity-rail 2>&1 | tail -15`
Expected: FAIL — module not found.

- [ ] **Step 3: Implement `ActivityRail`**

Create `web/src/components/ActivityRail.tsx`:

```tsx
import type { Item } from "../state";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ActivityRail({ items, sessionLabel, onOpenSettings, collapsed, onToggleCollapse }:
  { items: Item[]; sessionLabel: string; onOpenSettings?: () => void;
    collapsed: boolean; onToggleCollapse: () => void }) {
  const tools = items.filter((i): i is ToolItem => i.kind === "tool");
  return (
    <div className="flex h-full flex-col gap-2 p-2"
      style={{ width: collapsed ? 44 : 168, background: "var(--surface-raised)", borderRight: "1px solid var(--border)" }}>
      <div className="flex items-center justify-between">
        {!collapsed && <span className="text-xs font-semibold" style={{ color: "var(--text-strong)" }}>{sessionLabel}</span>}
        <button onClick={onToggleCollapse} aria-label="toggle activity rail"
          className="text-xs hover:opacity-80" style={{ color: "var(--text-muted)" }}>{collapsed ? "»" : "«"}</button>
      </div>
      {!collapsed && <div className="text-[10px] uppercase tracking-wide" style={{ color: "var(--text-muted)" }}>Activity</div>}
      <div className="flex flex-1 flex-col gap-1 overflow-y-auto">
        {tools.map((t, i) => (
          <div key={i} className="flex items-center gap-2 rounded px-1.5 py-1 text-xs" style={{ color: "var(--text)" }}>
            <span className="h-1.5 w-1.5 flex-none rounded-full"
              style={{ background: t.status === "running" ? "var(--state-run)" : "var(--state-done)" }} />
            {!collapsed && <span className="truncate font-mono">{t.name}</span>}
          </div>
        ))}
      </div>
      {onOpenSettings && (
        <button onClick={onOpenSettings} className="flex items-center gap-2 rounded px-1.5 py-1 text-xs hover:opacity-80"
          style={{ color: "var(--text-muted)" }} aria-label="settings">
          <span>⚙</span>{!collapsed && <span>Settings</span>}
        </button>
      )}
    </div>
  );
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run (from `web/`): `npm test -- activity-rail 2>&1 | tail -15`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add web/src/components/ActivityRail.tsx web/test/activity-rail.test.tsx
git commit -m "feat(web): ActivityRail listing live tool activity"
```

---

### Task 12: Assemble the three-pane shell in `App` (retire `TimelineView`)

**Files:**
- Modify: `web/src/App.tsx`
- Delete: `web/src/components/TimelineView.tsx`
- Test: `web/test/app-shell.test.tsx` (create)

**Interfaces:**
- Consumes: `ActivityRail` (Task 11), `Inspector` (Task 10), `artifactsFrom` (Task 7).
- Produces: `App` renders `StatusBar` over a row of `[ActivityRail | conversation+composer | Inspector]`; manages `activeArtifactKey` (useState) + rail/inspector collapse state; auto-selects the latest artifact when a new one arrives.

- [ ] **Step 1: Write the failing test**

Create `web/test/app-shell.test.tsx`:

```tsx
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { ActivityRail } from "../src/components/ActivityRail";
import { Inspector } from "../src/components/inspector/Inspector";
import { artifactsFrom, type Item } from "../src/state";

// Shell wiring is integration-level; this test asserts the pieces compose:
// artifactsFrom feeds Inspector, and ActivityRail + Inspector render side by side.
describe("workbench shell pieces compose", () => {
  it("derives artifacts and feeds them to the Inspector", () => {
    const items: Item[] = [
      { kind: "tool", name: "render", args: {}, status: "done",
        display: { Markdown: { text: "# Hello inspector", title: "Doc" } } },
    ];
    const arts = artifactsFrom(items);
    render(
      <div style={{ display: "flex" }}>
        <ActivityRail items={items} sessionLabel="s" collapsed={false} onToggleCollapse={() => {}} />
        <Inspector artifacts={arts} activeKey={arts[0].key} onSelect={() => {}} onClose={() => {}} />
      </div>
    );
    expect(screen.getByRole("tab", { name: "Doc" })).toBeInTheDocument();
    expect(screen.getByText("Hello inspector")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run the test to verify it fails (or passes trivially), then proceed to the shell change**

Run (from `web/`): `npm test -- app-shell 2>&1 | tail -15`
Expected: PASS (the imported pieces exist from prior tasks). This test guards composition; the real change is the `App` rewrite below — after it, run the full suite in Step 5.

- [ ] **Step 3: Rewrite `App.tsx`'s render to the three-pane shell**

In `web/src/App.tsx`:
- Remove the import of `TimelineView` and the `import { MessageList }` stays.
- Add imports:
  ```tsx
  import { ActivityRail } from "./components/ActivityRail";
  import { Inspector } from "./components/inspector/Inspector";
  import { artifactsFrom } from "./state";
  ```
- Remove `const turns = useTurnGrouping(animatedItems);` (TimelineView was its only consumer) and the `useTurnGrouping` import.
- Add state near the other `useState` calls:
  ```tsx
  const [activeArtifactKey, setActiveArtifactKey] = useState<string | null>(null);
  const [railCollapsed, setRailCollapsed] = useState(false);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  const artifacts = artifactsFrom(state.items);
  useEffect(() => {
    if (artifacts.length > 0) { setActiveArtifactKey(artifacts[artifacts.length - 1].key); setInspectorOpen(true); }
  }, [artifacts.length]);
  ```
- Replace the connected `return (...)` block's body (the part after `<StatusBar .../>` and the settings panel) with:
  ```tsx
      <div className="flex min-h-0 flex-1">
        <ActivityRail items={state.items} sessionLabel={sessionId.slice(0, 8)}
          onOpenSettings={openSettings} collapsed={railCollapsed}
          onToggleCollapse={() => setRailCollapsed((c) => !c)} />
        <div ref={messageListRef} className="flex min-w-0 flex-1 flex-col overflow-y-auto">
          <MessageList items={animatedItems} />
        </div>
        {inspectorOpen && (
          <div style={{ width: 360, borderLeft: "1px solid var(--border)" }} className="min-h-0">
            <Inspector artifacts={artifacts} activeKey={activeArtifactKey}
              onSelect={setActiveArtifactKey} onClose={() => setInspectorOpen(false)} />
          </div>
        )}
      </div>
      {state.pendingApproval && <ApprovalPrompt approval={state.pendingApproval} onDecide={decide} />}
      <Composer disabled={!connected} onSend={send} />
  ```
  (Keep the outer `<div className="flex h-screen flex-col" style={{ background: "var(--surface-base)" }}>` and the `<StatusBar .../>` + settings panel above it.)

- [ ] **Step 4: Delete `TimelineView`**

```bash
git rm web/src/components/TimelineView.tsx
```
Then search for any leftover references: `grep -rn "TimelineView\|useTurnGrouping" web/src web/test`. Remove the now-unused `useTurnGrouping`/`turnGroupsFrom` only if nothing references them — if `web/test` still tests `turnGroupsFrom`, leave the helper in `state.ts` (it's harmless) and only remove the `App` usage. Do **not** delete tested helpers.

- [ ] **Step 5: Run the full suite + typecheck**

Run (from `web/`): `npm test 2>&1 | tail -20 && npm run typecheck 2>&1 | tail -10`
Expected: all tests PASS, typecheck clean. If a `TimelineView` test file exists and now fails to import, delete that test file (`git rm web/test/<timeline-test>.tsx`) since the component is retired.

- [ ] **Step 6: Commit**

```bash
git add -A web/src web/test
git commit -m "feat(web): assemble three-pane Workbench shell, retire TimelineView"
```

---

### Task 13: Restyle conversation, tool chips, Composer to tokens

**Files:**
- Modify: `web/src/components/MessageList.tsx`
- Modify: `web/src/components/AssistantMessage.tsx`
- Modify: `web/src/components/ReasoningMessage.tsx`
- Modify: `web/src/components/ToolCall.tsx`
- Modify: `web/src/components/Composer.tsx`
- Test: `web/test/tool-components.test.tsx` (existing — keep green)

**Interfaces:**
- Consumes: theme tokens (Task 5).
- Produces: tool calls render as compact chips using tokens; user/assistant bubbles + composer use tokens. No prop/signature changes (so existing tests keep passing).

- [ ] **Step 1: Confirm the existing tests are the guardrail**

Run (from `web/`): `npm test -- tool-components 2>&1 | tail -10`
Expected: PASS now. These assert text content (`execute_command`, `ls`, diff lines), not classes — so restyling must keep that text. Keep them green throughout.

- [ ] **Step 2: Restyle `ToolCall.tsx` as a chip**

Replace `web/src/components/ToolCall.tsx`:

```tsx
import type { Item } from "../state";

type ToolItem = Extract<Item, { kind: "tool" }>;

export function ToolCall({ item }: { item: ToolItem }) {
  const running = item.status === "running";
  return (
    <div className="my-1 inline-flex items-center gap-2 rounded-md px-2 py-1 font-mono text-xs"
      style={{ background: "var(--surface-raised)", border: "1px solid var(--border)", color: "var(--text)" }}>
      <span className="rounded-full px-1.5 text-[10px]"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>{item.name}</span>
      <span style={{ color: running ? "var(--state-run)" : "var(--state-done)" }}>{running ? "…" : "✓"}</span>
    </div>
  );
}
```

Note: the rich rendering (Diff/Terminal/Text) that `ToolCall` previously did inline now lives in the Inspector (Task 8). The existing `tool-components` test renders `ToolCall` directly and expects the tool name + `ls` from a Terminal display. **To keep that test valid**, update its third assertion: that test should now assert the chip shows `execute_command` (the name). Edit `web/test/tool-components.test.tsx`'s third case to drop the `/ls/` expectation (the terminal body is no longer shown by the chip):

```tsx
  it("ToolCall renders a compact chip with the tool name and status", () => {
    render(<ToolCall item={{ kind: "tool", name: "execute_command", args: {}, status: "done", content: "exit=0",
      display: { Terminal: { command: "ls", stdout: "file\n", stderr: "", exit_code: 0 } } }} />);
    expect(screen.getByText(/execute_command/)).toBeInTheDocument();
    expect(screen.getByText("✓")).toBeInTheDocument();
  });
```

(The `DiffView`/`TerminalBlock` direct-render cases in that file stay unchanged — those components are unchanged and still used by the Inspector.)

- [ ] **Step 3: Restyle the message components**

Replace `web/src/components/AssistantMessage.tsx`:

```tsx
import type { Item } from "../state";
import { MarkdownText } from "./MarkdownText";

export function AssistantMessage({ item }: { item: Extract<Item, { kind: "assistant" }> }) {
  return <div className="py-2" style={{ color: "var(--text)" }}><MarkdownText text={item.text} /></div>;
}
```

In `web/src/components/MessageList.tsx`, replace the user bubble line so it uses tokens:

```tsx
          case "user":
            return <div key={i} className="my-2 ml-auto max-w-[80%] rounded-lg px-3 py-2"
              style={{ background: "var(--text-strong)", color: "var(--surface-base)" }}>{it.text}</div>;
```

In `web/src/components/ReasoningMessage.tsx`, change its container color styling to `style={{ color: "var(--text-muted)" }}` (replace any `text-zinc-*` class with the inline token style; keep its existing structure/props).

- [ ] **Step 4: Restyle `Composer.tsx`**

Replace `web/src/components/Composer.tsx`:

```tsx
import { useState } from "react";

export function Composer({ disabled, onSend }: { disabled: boolean; onSend: (text: string) => void }) {
  const [text, setText] = useState("");
  const submit = () => {
    const t = text.trim();
    if (!t || disabled) return;
    onSend(t);
    setText("");
  };
  return (
    <div className="flex gap-2 p-3" style={{ background: "var(--surface-base)", borderTop: "1px solid var(--border)" }}>
      <textarea
        className="flex-1 resize-none rounded-lg p-2 outline-none disabled:opacity-50"
        style={{ background: "var(--surface-overlay)", color: "var(--text-strong)", border: "1px solid var(--border)" }}
        rows={2}
        value={text}
        disabled={disabled}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submit(); } }}
        placeholder={disabled ? "disconnected…" : "Message the agent…"}
      />
      <button onClick={submit} disabled={disabled}
        className="rounded-lg px-4 disabled:opacity-50 hover:opacity-90"
        style={{ background: "var(--accent)", color: "var(--accent-fg)" }}>Send</button>
    </div>
  );
}
```

- [ ] **Step 5: Run the full suite + typecheck**

Run (from `web/`): `npm test 2>&1 | tail -20 && npm run typecheck 2>&1 | tail -10`
Expected: all PASS, typecheck clean.

- [ ] **Step 6: Commit**

```bash
git add web/src/components web/test/tool-components.test.tsx
git commit -m "feat(web): restyle conversation, tool chips, and composer to tokens"
```

---

### Task 14: Restyle `PairingScreen`, `ApprovalPrompt`, `SettingsPanel`, `TerminalBlock`

**Files:**
- Modify: `web/src/components/PairingScreen.tsx`
- Modify: `web/src/components/ApprovalPrompt.tsx`
- Modify: `web/src/components/TerminalBlock.tsx`
- Modify: `web/src/components/SettingsPanel.tsx`
- Test: existing `web/test/*` (keep green); `web/test/shell-components.test.tsx`

**Interfaces:**
- Consumes: theme tokens.
- Produces: these surfaces use tokens (no `zinc-*`/hardcoded colors). No prop changes.

- [ ] **Step 1: Confirm guardrail tests pass**

Run (from `web/`): `npm test -- shell-components 2>&1 | tail -10`
Expected: PASS. (These assert text/roles, not colors.)

- [ ] **Step 2: Restyle `PairingScreen.tsx`**

In `web/src/components/PairingScreen.tsx`, replace the color classes: the container `text-zinc-100` → `style={{ color: "var(--text-strong)" }}`; the input's `bg-zinc-900 ... text-...` → `style={{ background: "var(--surface-overlay)", color: "var(--text-strong)", border: "1px solid var(--border)" }}`; the button `bg-zinc-700 hover:bg-zinc-600` → `style={{ background: "var(--accent)", color: "var(--accent-fg)" }}`; the error `text-red-400` → `style={{ color: "var(--state-error)" }}`. Keep all logic/handlers identical.

- [ ] **Step 3: Restyle `ApprovalPrompt.tsx`**

Replace `web/src/components/ApprovalPrompt.tsx`:

```tsx
import type { Decision } from "../wire";
import type { PendingApproval } from "../state";

export function ApprovalPrompt({ approval, onDecide }: { approval: PendingApproval; onDecide: (d: Decision) => void }) {
  return (
    <div className="mx-4 my-2 rounded-lg p-3 text-sm"
      style={{ border: "1px solid var(--accent-2)", background: "var(--surface-raised)" }}>
      <div className="mb-2" style={{ color: "var(--text-strong)" }}>Allow: {approval.summary}</div>
      {approval.command && (
        <pre className="mb-2 overflow-x-auto font-mono" style={{ color: "var(--accent-2)" }}>{approval.command}</pre>
      )}
      <div className="flex gap-2">
        <button onClick={() => onDecide("approve")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--state-done)", color: "var(--accent-fg)" }}>Approve</button>
        <button onClick={() => onDecide("approve_always")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--surface-overlay)", color: "var(--text)", border: "1px solid var(--border)" }}>Approve always</button>
        <button onClick={() => onDecide("deny")} className="rounded px-3 py-1 hover:opacity-90"
          style={{ background: "var(--state-error)", color: "#fff" }}>Deny</button>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Restyle `TerminalBlock.tsx`**

Open `web/src/components/TerminalBlock.tsx` and replace its `zinc-*`/color classes with token inline styles: container `style={{ background: "var(--surface-overlay)", border: "1px solid var(--border)" }}`; command text `var(--accent)`; stdout `var(--text)`; stderr `var(--state-error)`; the `exit N` label `var(--text-muted)`. Keep the structure and the `exit {exitCode}` text exactly (the `tool-components` test asserts `/exit 0/`).

- [ ] **Step 5: Restyle `SettingsPanel.tsx`**

In `web/src/components/SettingsPanel.tsx`, replace `zinc-*` background/text/border classes with the token inline styles (`--surface-overlay` for the panel, `--surface-base` for inputs, `--text`/`--text-strong` for text, `--border` for borders, `--accent` for the primary save button, `--state-error` for error text). Do not change any field names, handlers, or the `onSave`/`onClose` props — only colors. Run its test after.

- [ ] **Step 6: Run the full suite + typecheck**

Run (from `web/`): `npm test 2>&1 | tail -20 && npm run typecheck 2>&1 | tail -10`
Expected: all PASS (including `SettingsPanel.test.tsx`), typecheck clean.

- [ ] **Step 7: Commit**

```bash
git add web/src/components
git commit -m "feat(web): restyle pairing, approval, terminal, and settings to tokens"
```

---

### Task 15: Final polish — build, manual smoke, motion guard

**Files:**
- Modify: `web/src/components/Animated*.tsx` (only if they carry hardcoded colors)
- Test: full suites + production build

**Interfaces:**
- Consumes: everything above.
- Produces: a clean production build and a verified light/dark, three-pane app with working artifact rendering.

- [ ] **Step 1: Scan for leftover hardcoded colors**

Run (from `web/`): `grep -rn "zinc-\|slate-\|bg-gray\|text-gray" src/ || echo "clean"`
Expected: ideally `clean`. For any remaining hit in a component, replace with the matching token (surface/text/border/accent). The `Animated*` wrappers mostly handle motion, not color — fix only real color classes; leave framer-motion props alone.

- [ ] **Step 2: Confirm reduced-motion is honored**

Confirm `web/src/index.css` contains the `@media (prefers-reduced-motion: reduce)` block from Task 5 (it globally clamps animation/transition durations). No code change if present.

- [ ] **Step 3: Run the entire frontend suite + typecheck + build**

Run (from `web/`): `npm test 2>&1 | tail -20 && npm run typecheck 2>&1 | tail -5 && npm run build 2>&1 | tail -15`
Expected: all tests PASS, typecheck clean, `vite build` succeeds (writes `dist/`).

- [ ] **Step 4: Run the entire backend suite**

Run: `source ~/.cargo/env && cargo test -p agent-tools -p agent-runtime-config -p agent-server 2>&1 | tail -15`
Expected: all green.

- [ ] **Step 5: Manual smoke (optional but recommended)**

Use the project's launch script (see `scripts/launch-web-ui.sh`) to bring up the stack, pair, send a message, and confirm: three panes render; theme toggle flips light/dark and persists across reload; a tool call appears in the ActivityRail and (if it carries a display) opens in the Inspector; ask the agent to call `render` with a markdown/mermaid artifact and confirm it shows in the Inspector. (Reference the E2E notes in memory if CORS/origin issues arise.)

- [ ] **Step 6: Commit any polish**

```bash
git add -A web/src
git commit -m "chore(web): final token cleanup and build verification"
```

---

## Self-Review

**1. Spec coverage:**
- §3 Design system → Task 5 (tokens, light/dark, reduced-motion), Task 6 (toggle wired).
- §4 Three-pane shell → Tasks 11 (rail), 10 (inspector), 12 (assembly), retires `TimelineView`.
- §5.1 Display extension → Task 1. §5.2 render tool → Task 2 + Task 3 (registration). §5.3 wire mirror + frontend → Task 4 (TS), Tasks 8-10 (renderers/inspector), Task 12 (routing via `artifactsFrom` + auto-open). §5.5 security (sandboxed HTML) → Task 9.
- §5.4 v2 → explicitly deferred (no task; documented in spec).
- §6 component inventory restyle → Tasks 13-14; reuse of DiffView/TerminalBlock/MarkdownText inside Inspector → Task 8/9. `TimelineView` retired → Task 12.
- §7 motion/states/testing → empty/offline states (Inspector empty Task 10; StatusBar offline already), reduced-motion (Task 5/15), tests in every task. Worker untouched invariant honored (no `cloud/` task).

**2. Placeholder scan:** No "TBD/TODO/handle edge cases/similar to Task N". Every code step shows full code. The only judgment calls (Task 12 Step 4 leftover-reference cleanup; Task 14/15 class-by-class swaps) give explicit rules and the exact text/behavior to preserve.

**3. Type consistency:** `Display` variant shapes match between Rust (Task 1) and TS (Task 4). `InspectorArtifact { key, title, display }` defined in Task 7 and consumed unchanged in Tasks 10/12. `ArtifactRenderer({ display })`, `Inspector({ artifacts, activeKey, onSelect, onClose })`, `ActivityRail({ items, sessionLabel, onOpenSettings, collapsed, onToggleCollapse })`, `StatusBar(+ theme, onToggleTheme)`, `RenderArtifact` (`name() == "render"`) — all consistent across the tasks that reference them. `exit_code` (wire) → `exitCode` (TerminalBlock prop) handled explicitly in Task 8.
