# Architecture Viewer — Design

**Date:** 2026-07-05
**Status:** Approved (brainstorm complete)
**Scope:** Stage 2 of the "Claude design" hub tab — the Architecture sub-section
**Prior stage:** `2026-07-05-claude-design-tab-design.md` (canvas + config, merged c54b47b)

## Context

The Design tab currently has two sub-sections (Canvas, Config). This stage adds the
third: an **architecture viewer** — the runtime's own design as living documentation.

## Decisions (made during brainstorm)

| decision | choice |
|----------|--------|
| Data source | **Live runtime self-portrait**: the daemon reports its actual wiring — not curated repo diagrams (stale) nor an agent-generated picture (imagination, not ground truth) |
| Interactivity | **Diagram + drill-down**: fixed block diagram; clicking a block opens a detail panel. No live per-turn telemetry (that is the Context tab's job) |
| Mechanism | **Approach A — introspection snapshot**: one read-only Tauri command `architecture_get` returning an `ArchitectureSnapshot`; client renders hand-laid-out React/SVG. Rejected: settings-derived Mermaid (cannot show registered tools; shows config, not reality), agent-generated (not ground truth) |
| Exposure | Tauri-gated like Config; nothing on the Worker path |
| Freshness | Fetch on sub-tab entry + manual refresh button. No push channel, no localStorage cache — staleness is the enemy, the fetch is cheap |

## Architecture overview

Design tab sub-nav becomes **Canvas | Config | Architecture** (Architecture
Tauri-gated, same guard as Config). `ArchitecturePane` fetches an
`ArchitectureSnapshot` via `architecture_get` and renders a fixed-layout block
diagram (Model → AgentLoop → ToolRegistry → Policy → Sandbox; Context Manager and
Prompt feeding the loop) with dynamic contents/badges and a click-to-open detail
panel.

The snapshot reflects the **live loop** (post-`apply`), so:
- a config change shows on next fetch;
- configured-but-unregistered tools (e.g. an MCP server that failed to connect)
  are visible as absences in the registered list — the gap between "configured"
  and "actual" is precisely what a self-portrait must reveal.

## Snapshot content (blocks)

- **Model**: backend, base_url (redacted to scheme+host), model, protocol,
  sampling summary (temperature/top_p/top_k), thinking flags.
- **Tools**: registered tool list from the live loop's schemas (name + first
  sentence of description), partitioned builtin / MCP / memory / skills /
  context; subagent dispatch availability.
- **Policy**: command allowlist, effective denylist (hard floor marked),
  HTTP allow hosts.
- **Sandbox**: live `SandboxDescriptor` — mechanism, image, network, degraded
  reason if any.
- **Context**: context limit, max tool result bytes, offload/compaction posture,
  compaction model if routed, memory on/off, recall budget.
- **Loop**: max turns, max parallel tools, stream idle timeout, subagent
  settings (enabled, max depth, routed model).
- **Prompt**: composed system-prompt token estimate + override active flag
  (boolean + length only — never the text; Config owns editing).

## Components

### Rust

| unit | responsibility |
|------|----------------|
| `agent-server/src/wire.rs`: `ArchitectureSnapshot` + `ModelInfo`, `ToolEntry {name, summary, kind}`, `PolicyInfo`, `SandboxInfo`, `ContextInfo`, `LoopInfo`, `PromptInfo` | Serializable snapshot beside `SettingsState` (crosses the IPC boundary) |
| `agent-server/src/runtime.rs`: `RuntimeState::architecture()` | Assembles snapshot from `self.config`, `current_loop()` (tool schemas, sandbox descriptor), `current_system_prompt()` (estimate + override flag) |
| `agent-server/src/session.rs`: `Session::architecture()` | Delegation mirroring `settings_get()`; contributes `recall_budget` (a `Session` field, not held by `RuntimeState`) into the Context block |
| `src-tauri/src/lib.rs`: `architecture_get` | One-liner command registered beside `settings_get` |
| `agent-core/src/loop_.rs`: `pub fn tool_schemas(&self)` | Accessor exposing registered schemas (field exists; new public accessor) |

Tool partitioning: `RuntimeState` classifies by provenance it already holds —
name-set membership in the `mcp_tools` / `memory_tools` arcs it was constructed
with, context tools by known names, remainder = builtin. No `agent-tools` changes.

### Web (`web/src/components/design/`)

| unit | responsibility |
|------|----------------|
| `ArchitecturePane.tsx` | Fetch on mount/re-entry + refresh button; loading/error/retry states; `selectedBlock` state; lays out diagram + detail |
| `ArchDiagram.tsx` | Pure SVG: fixed block layout + arrows, dynamic badges (tool count, "degraded", "override active"); `onSelect(blockId)` |
| `ArchDetail.tsx` | Selected block's slice as definition list (tools table, policy lists with hard-floor marks) |
| `architecture.ts` | TS mirror of the snapshot + `fetchArchitecture()` (`invoke("architecture_get")`) |

`DesignPane.tsx`: sub-nav array gains `"architecture"` (Tauri-gated).
Boundaries: `ArchDiagram`/`ArchDetail` are pure (snapshot in, callbacks out);
only `architecture.ts` touches `invoke`.

## Data flow

Open sub-tab → `fetchArchitecture()` → `architecture_get` →
`Session::architecture()` → `RuntimeState::architecture()` reads live loop Arc +
config under existing lock discipline (held only for clones, as `current_loop()`)
→ snapshot returns → diagram renders. Sub-tab re-entry and the refresh button
re-fetch. Block click sets `selectedBlock`; `ArchDetail` renders that slice.
No localStorage caching.

## Error handling

- `invoke` failure: inline error + retry button; never render a stale snapshot
  as fresh.
- Redaction: base_url → scheme+host only; prompt text never leaves the daemon
  (estimate + flag only). Snapshot is safe to screenshot/share.
- Degraded sandbox: badge on the Sandbox block; reason in the detail panel
  (reuses `SandboxDescriptor.degraded`).
- Tool classification is best-effort: unclassifiable tools land in "builtin",
  never dropped — the list is always complete.

## Testing

- **Rust** (`agent-server`): snapshot contains registered tool names; MCP/memory
  partitioning with injected tool arcs; hard floor present in effective denylist;
  degraded sandbox propagates; override flag tracks config; base_url redaction
  golden test. `src-tauri`: command smoke test (own workspace).
- **Web** (vitest): `ArchDiagram` renders all seven blocks + badges from a
  fixture; click fires `onSelect`; `ArchDetail` tools table + hard-floor marks;
  `ArchitecturePane` loading→data and error→retry (mock `architecture.ts`);
  `DesignPane` shows Architecture sub-tab under Tauri only.
- **Manual**: live tab shows real tool count; change a config value in Config,
  refresh Architecture, see it reflected.

## Out of scope

- Live per-turn telemetry overlays (Context tab's job).
- Editing anything from this view (Config owns mutation).
- Worker/browser exposure (Tauri-gated, consistent with plan deviation #4 of
  stage 1).
- Repo-code architecture (graphify's job, `graphify-out/graph.html`).
