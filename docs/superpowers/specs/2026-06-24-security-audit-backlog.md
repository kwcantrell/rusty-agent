# Security/Robustness Audit Backlog — 2026-06-24

Source: graphify-guided audit of the agent runtime (four parallel crate audits, findings
verified against code). The **tool-execution security boundary** cluster was promoted to
its own spec (`2026-06-24-tool-execution-security-boundary-design.md`). The clusters below
are the remaining verified findings, not yet specced.

> Line references are accurate as of commit `a8f1b47` (post memory-auto-retrieval merge).
> The loop-robustness cluster touches files the merge reworked — re-verify before specing.

## Cluster A — Server concurrency & resource leaks (`agent-server`, `agent-mcp`)

> Decomposed (2026-06-25) into 3 sub-specs: **A-a** bounded event channel + backpressure
> (finding A2), **A-b** MCP process & task lifecycle (finding A3), **A-c** run lifecycle /
> session identity / atomic `RuntimeState` + deferred B3 interactive server-cancel
> (findings A1 + A4 + B3-cancel).
> **A-b DONE** (merged `6d23647`, 2026-06-25): `StdioTransport` stores + aborts its
> stdout-reader/stderr-drain `JoinHandle`s on `close()`/`Drop`; `SandboxedChild::kill()`
> reaps via explicit `wait()` after `start_kill()`. Spec
> `2026-06-25-mcp-process-lifecycle-design.md`, plan `…/plans/2026-06-25-mcp-process-lifecycle.md`.
> (Honest framing: hardened already-`kill_on_drop`-mitigated teardown to be deterministic;
> not an unbounded leak — no reconnect path exists.)
> **A-a DONE — by dissolution** (branch `feat/tauri-ipc-transport`, 2026-06-25): rather than
> bound a cloud-shaped channel, the loopback WebSocket transport was replaced with native
> Tauri IPC (typed `#[command]`s in + one live-read `ipc::Channel<ServerEvent>` out). There is
> no longer an `mpsc` event channel or writer task, so **A2 cannot recur**. The migration also
> closed **A1** (single-active-run guard rejects concurrent `user_input` with `Busy`) and the
> **deferred B3 interactive server-cancel** (`cancel()` command trips the run's
> `CancellationToken`). `session_id` identity (part of A-c) is gone as a side effect — the wire
> envelope/`PROTOCOL_VERSION`/`session_id` were deleted. Spec
> `2026-06-25-tauri-ipc-transport-design.md`, plan `…/plans/2026-06-25-tauri-ipc-transport.md`.
> **Remaining: A-c reduced to the `RuntimeState` three-mutex atomicity refactor only**
> (finding A4); A1 + identity already resolved here.

- **HIGH — concurrent `user_input` session/approval cross-talk** (`agent-server/src/daemon.rs:~104`).
  ~~Each input spawns a detached task sharing one session id + one global approval-correlation
  space; a second input mid-run misattributes frames. No guard rejects a second concurrent run.~~
  **RESOLVED (A-a, Tauri IPC):** `send_input` is single-active-run-guarded (`Busy` on a second
  concurrent input); `session_id` removed entirely.
- **HIGH — unbounded event channel (memory-exhaustion DoS)** (`daemon.rs:~44`, `sink.rs:~9`).
  ~~`mpsc::unbounded_channel` carries every token/chunk/frame; `emit` is infallible. A slow/stalled
  WebSocket client lets the queue grow without bound.~~ **RESOLVED by dissolution (A-a):** the WS
  transport + `mpsc` channel were removed; events now stream over a Tauri `ipc::Channel`.
- **HIGH — MCP stdio reader tasks never aborted; children killed but never `wait()`ed**
  (`agent-mcp/src/transport.rs:~53-97`). Reconnect cycles leak detached tasks and zombie processes.
- **MED — `RuntimeState` god object with three independent mutexes updated non-atomically**
  (`agent-server/src/runtime.rs:~18-33,90-92`). Permits torn reads of the config/loop/prompt triple.

## Cluster B — Loop robustness (`agent-core`, `agent-model`) — RE-VERIFY LINES

> **B1 DONE** (merged `6383a7b`, 2026-06-25): the HIGH tool-call-id panic + the LOW
> `message_tokens` undercount are fixed (spec `2026-06-25-loop-tool-call-id-contract-design.md`,
> plan `docs/superpowers/plans/2026-06-25-loop-tool-call-id-contract.md`).
> **B2 DONE** (merged `ee74971`, 2026-06-25): OpenAI stream robustness — truncation
> detection, skip-malformed-SSE-line, in-band 200-body error surfacing (spec
> `2026-06-25-openai-stream-robustness-design.md`, plan
> `…/plans/2026-06-25-openai-stream-robustness.md`).
> **B3 DONE** (merged `9e5681f`, 2026-06-25): live cancellation — caller-owned
> `CancellationToken` through `run_with_cancel` (turn boundary + model-stream
> select + every `ToolCtx`), `Done(StopReason::Cancelled)` on cancel, CLI Ctrl-C
> source; interactive server cancel deferred to Cluster A (spec
> `2026-06-25-live-cancellation-design.md`, plan `…/plans/2026-06-25-live-cancellation.md`).
> **Cluster B is COMPLETE.** Only Cluster A remains.

- **HIGH — panic on duplicate/empty tool-call ids** (`agent-core/src/loop_.rs`, the
  `results.remove(...).expect("every gated call id has a result")` path). `results` is a HashMap
  keyed by id; colliding/empty ids (model-controllable; passthrough protocol defaults to `"c"`)
  cause a panic in the hot path.
- **MED — OpenAI stream fragility** (`agent-model/src/openai.rs`). No `Chunk::Done` on a
  truncated/`[DONE]`-less stream (silent truncation looks complete); one malformed SSE `data:`
  line aborts the whole stream; in-band `{"error":...}` in a 200 body is swallowed.
- **MED — cancellation token is inert** (`agent-core/src/loop_.rs`). No Ctrl-C / tool cancellation;
  hung tools in the parallel set can't be aborted mid-turn.
- **LOW — `message_tokens` undercounts** (`agent-core/src/context.rs`): ignores `reasoning` and
  `tool_calls`, so the window-eviction budget and `Usage` events underestimate; amplified by
  auto-preserve. (Long-standing; known.)

## Cluster C — HTTP redirect policy bypass (`agent-http`) — PROMOTED TO SPEC

> Promoted (2026-06-25) to `2026-06-25-policy-boundary-consistency-design.md`,
> together with a newly-found sibling: the read-path approval gate in
> `agent-policy/src/engine.rs:43-50` uses a non-normalizing `starts_with`, so a
> `../`-escaping relative read returns `Decision::Allow` and skips the approval
> prompt (execute() still hard-blocks via `resolve_in_workspace`). Same root
> cause — fixed by reusing `resolve_in_workspace` for the decision.

- **HIGH — redirects bypass the allowlist/approval policy** (`agent-http/src/tool.rs:~190-207`).
  Approval is computed from the original URL's host; the redirect loop re-runs only the SSRF guard,
  never `policy.decide()`. An approved fetch to an allowlisted host can 302 to any public host.
  Same root cause as the boundary spec ("decide once, execute elsewhere"); deferred there as a
  separate small spec — re-run the host policy decision on every hop.

## Notes — verified NOT issues (don't re-litigate)

- graphify "import cycles" are all 1-file self-references (extraction artifact), not real cycles.
- graphify top god node `base()` is a duplicated `#[cfg(test)]` fixture, not production code.
- SSRF guard correctly handles IPv4-mapped IPv6, cloud-metadata `169.254.169.254`, and
  decimal/octal/hex IP encodings (normalized before the guard).
- Config save/load round-trip is sound for currently-defined fields (covered by round-trip tests);
  the residual hazard is the hand-written 30-field `RuntimeConfig::merge` requiring edits in 5 places.
