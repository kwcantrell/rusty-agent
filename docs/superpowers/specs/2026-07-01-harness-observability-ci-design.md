# Harness Observability + CI — Design

**Date:** 2026-07-01
**Source:** Cluster 2 of the harness deep audit
(`docs/superpowers/audits/2026-07-01-harness-deep-audit.md`) — "the harness
can't see itself." Closes audit findings: Observability HIGH×3 (silent tool
failures, no durations, no persistence), Observability MED×3 (ContextEvent
dropped, claude-cli usage 0/0, reasoning tokens uncounted, no id correlation),
Observability LOW (no session aggregates), Eval HIGH (no CI).
**Approach:** enrich the existing `AgentEvent` spine in place (approved over a
parallel `tracing`-based channel and over purely-additive events) — one source
of truth, one-time breaking enum migration, both frontends benefit for free.

## Goals

1. Every tool call has a visible, correlated lifecycle: `ToolStart{id}` →
   exactly one `ToolResult{id, status, duration_ms}` — including denials,
   errors, timeouts, and panics, which today emit nothing.
2. Every session is replayable: a JSONL trace persisted by default on all
   surfaces (CLI, server, desktop), so a failed turn can be diagnosed — and
   harvested into eval tasks — without re-running.
3. Context curation is observable: offload/compaction/compaction-failure events
   reach both UIs instead of being dropped.
4. Token/cost accounting is faithful: reasoning + cached tokens (OpenAI-compat)
   and tokens + `total_cost_usd` (claude-cli, currently reporting 0/0) flow
   through `ServerUsage`.
5. Per-session aggregates (tokens, cost, tool-error rate, durations) are
   queryable mid-session and rendered in a web Context Explorer stats panel.
6. A CI gate (local hook + GitHub Actions) runs fmt/clippy/tests/web checks on
   every change.

## Non-goals

- Fixing the three Done-less terminal paths (orchestration cluster).
- Persisted OffloadStore / durable session artifacts (context cluster).
- Trace redaction (traces are local files in `~/.agent`; revisit if traces
  ever leave the machine).
- Drift *alerting* — the dashboard shows numbers, it does not judge them.
- `src-tauri` in CI (needs GTK/WebKitGTK system deps; follow-up).

## 1. Event model (`agent-core/src/event.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus { Ok, Denied, Error, Timeout, Panic }
```

- `ToolStart { id: String, name: String, args: serde_json::Value }` — gains
  `id` (the tool_call id; already unique per turn via
  `normalize_tool_call_ids`).
- `ToolResult { id: String, name: String, status: ToolStatus, output: ToolOutput, duration_ms: u64 }`
  — gains `id`, `status`, `duration_ms`. For non-Ok statuses `output` carries
  the error text already fed back to the model (`ERROR: …`).
- `ServerUsage { prompt_tokens: u32, completion_tokens: u32, reasoning_tokens: Option<u32>, cached_tokens: Option<u32>, cost_usd: Option<f64>, turn_duration_ms: u64, turn: usize }`.
- `ContextEvent` unchanged.
- `Denied` covers all gate rejections (unknown tool, bad intent, policy deny,
  user decline); the distinction stays in the output text, not the enum —
  YAGNI until an eval needs it.

**Invariant (tested):** every emitted `ToolStart` is followed by exactly one
`ToolResult` with the same `id`, for every status.

## 2. Emission points (`agent-core/src/loop_.rs`)

- Phase-2 execute closure: capture `std::time::Instant` around
  `execute_isolated`; thread elapsed into `Resolved` (both arms — `Ok` and
  `Err` now carry `(content, duration_ms)`).
- Phase-3 drain (single choke point, currently ~331-352): emit one
  `ToolResult` per resolved call — replacing today's Ok-only emission. Gate
  rejections (which never execute) emit with `duration_ms: 0`.
- Status mapping: `Executed::Output` → `Ok`; `Executed::ToolErr` → `Error`;
  timeout backstop → `Timeout`; panic isolation → `Panic`; `GateOutcome::Rejected`
  → `Denied`.
- `one_completion` is timed; elapsed lands in `ServerUsage.turn_duration_ms`.
  (Turns without server usage still emit `ServerUsage` with zeroed token fields
  today via the `AssistantTurn` default — unchanged; duration is now real.)

## 3. Usage parsing (`agent-model`)

- `Chunk::Usage` grows the same optional fields as `ServerUsage`.
- `openai.rs`: parse `usage.completion_tokens_details.reasoning_tokens` and
  `usage.prompt_tokens_details.cached_tokens` when present; absent → `None`.
  No behavior change for servers that omit the detail objects.
- `claude_cli.rs` (`parse_event_line`): parse the stream-json `result` event's
  `usage` block — `input_tokens` → prompt, `output_tokens` → completion — and
  top-level `total_cost_usd` → `cost_usd`; emit `Chunk::Usage`. Fixes the
  current 0/0 `ServerUsage` on claude-cli sessions and adds the system's only
  dollar-cost signal.

## 4. Trace persistence (`agent-runtime-config/src/trace.rs`, new)

`JsonlTraceSink` implements `EventSink`, tees: forwards every event to the
inner sink unchanged, and appends a mapped record to the trace file.

- **Record schema (stable, versioned; decoupled from the internal enum —
  mirrors the `wire.rs` mapping pattern):** first line
  `{"schema":1,"session":"<id>","started_ms":<epoch>}`, then one
  `TraceRecord { seq: u64, ts_ms: u64, event: TraceEvent }` per line, where
  `TraceEvent` is a serializable mirror of `AgentEvent` (Approval mapped to
  `{summary, command}`; `ToolOutput` mapped to `{content, display?}`).
- **Path:** `<trace_dir>/<session-id>.jsonl`, default trace_dir
  `~/.agent/sessions/`. No session id exists in the codebase; the sink mints
  `YYYYMMDD-HHMMSS-<pid>` at construction. The server constructs one sink per
  daemon session (reused across runs); the CLI one per invocation.
- **Config (`RuntimeConfig`):** `trace: bool` (default **true**),
  `trace_dir: Option<String>`, `trace_max_mb: u64` (default 64).
- **Cap & retention:** on size breach, stop writing, `tracing::warn!` once,
  keep running. At construction, prune the trace dir to the newest 50 files.
- **Error posture:** the trace sink must never fail a run. All I/O errors →
  one `tracing::warn!`, then writes disabled for the session.
- **Flush:** buffered writer, flushed on `Done` and `Error` events — a crash
  loses at most the current turn.
- **Wiring:** once, in `assemble_loop`
  (`parts.sink` → `StatsSink` → `JsonlTraceSink` → inner), covering CLI,
  server, and desktop with zero per-frontend code.

## 5. Wire + CLI forwarding

- `agent-server/src/wire.rs`:
  - `ServerEvent::ToolStart`/`ToolResult` gain `id`, `status: String`,
    `duration_ms` (status serialized snake_case to match the Rust enum).
  - New `ServerEvent::Context { kind: String, detail: serde_json::Value }`
    mapping all three `ContextEvent` variants; the
    `AgentEvent::Context(_) => return None` drop arm is removed.
  - `ServerUsage` passes the new optional fields through
    (`skip_serializing_if = "Option::is_none"`).
- `agent-cli/src/render.rs`:
  - Failed tools: one-liner `✗ read_file (timeout, 30012ms)`.
  - Context events: notice line, e.g. `⟲ compacted 12 turns: 41k → 9k tokens`,
    `⟲ offloaded tool result #4 (18 KB)`, `⚠ compaction failed: <reason>`.
  - End of run: stats summary line (from §6).

## 6. SessionStats + web dashboard

- `SessionStats` (in `agent-core`, plain struct + pure
  `fn fold(&mut self, &AgentEvent)`): cumulative prompt/completion/reasoning/
  cached tokens, `cost_usd`, turns, tool calls, per-`ToolStatus` counts,
  total tool time ms, wall-time ms, context events count.
- `StatsSink` tee layer updates `Arc<RwLock<SessionStats>>`; `assemble_loop`
  returns the handle in `BuiltLoop`.
- Server: new `session_stats` IPC query returns the snapshot; also pushed as a
  `ServerEvent::SessionStats` after each `Done` so an attached browser needs no
  poll. Mid-session attach gets correct totals from the query (no event
  replay needed).
- Web (`web/`): extend `wire.ts` types; reducer in `state.ts`; stats panel in
  the Context Explorer (tokens incl. reasoning/cached, cost, tool-error rate,
  tool/turn durations); context-event markers on the existing prompt-token
  chart so its token drops are finally explained.
- CLI: prints the same snapshot as a one-line summary at run end.

## 7. CI

- `scripts/ci.sh` (single source of truth for the gate):
  1. `cd agent && cargo fmt --all --check`
  2. `cargo clippy --workspace --all-targets -- -D warnings`
  3. `cargo test --workspace`
  4. `cd web && npm ci && npm run typecheck && npx vitest run`
- `.githooks/pre-push` runs `scripts/ci.sh`;
  `git config core.hooksPath .githooks` documented in CLAUDE.md (Commands
  section).
- `.github/workflows/ci.yml`: checkout + Rust toolchain + Node 20 + cargo
  cache, then `scripts/ci.sh` on push/PR. Inert until the repo gains a GitHub
  remote — becomes active that day with no further work.
- The deterministic suites (`e2e_robustness.rs`, `e2e_context_management.rs`)
  run as part of `cargo test --workspace` — they were built "CI-runnable" and
  finally are.

## Error handling summary

| Failure | Behavior |
|---|---|
| Trace dir unwritable / disk full / cap hit | warn once, disable trace writes, run continues |
| Stats lock poisoned | ignore (stats are advisory), run continues |
| Usage details absent (older servers) | fields `None`, no error |
| claude-cli result line unparseable | skip usage, warn, run continues |
| ci.sh step fails | nonzero exit → push blocked / workflow red |

## Testing

Spec-named tests (per crate, following repo convention):

- `agent-core/loop_.rs`: `every_resolved_call_emits_tool_result` (all five
  statuses, via testkit `CollectingSink`); `tool_result_ids_match_tool_start`;
  `executed_calls_report_nonzero_duration`; `denied_calls_report_zero_duration`;
  `server_usage_carries_turn_duration`.
- `agent-core`: `session_stats_fold_accumulates` (tokens, cost, per-status
  counts).
- `agent-model`: `openai_parses_reasoning_and_cached_tokens` (fixture with and
  without detail objects); `claude_cli_parses_result_usage_and_cost` (fixture
  stream-json `result` line).
- `agent-runtime-config/trace.rs`: `trace_writes_parseable_jsonl`;
  `trace_header_carries_schema_and_session`; `trace_respects_size_cap`;
  `trace_prunes_to_retention`; `trace_survives_unwritable_dir` (read-only
  tempdir: run completes, no panic); `assemble_wires_stats_and_trace_sinks`.
- `agent-server/wire.rs`: `context_events_are_forwarded` (all three variants);
  `tool_result_wire_carries_status_and_duration`; round-trip serde.
- e2e (`agent-runtime-config/tests/e2e_robustness.rs`): assert a denied and a
  timed-out call each surface a wire `ToolResult` with correct status.
- Web: reducer unit tests (stats accumulation, context markers) + panel render
  test via vitest.
- CI proves itself: the workflow/hook runs the full suite above.

## Migration notes

Breaking enum change: every `match` on `AgentEvent`/`ServerEvent` updates in
this PR — known sites: `agent-cli/render.rs`, `agent-server/wire.rs`,
`agent-server/sink.rs`, `agent-core/testkit.rs`, eval sinks in
`agent-runtime-config/tests/`, web `wire.ts`/`state.ts`. The compiler
enumerates the rest (no wildcard arms exist on these matches today — verify
and remove any found).
