# Claude CLI as inference backend — Phase 0 spike notes

Spike for `docs/superpowers/specs/2026-06-23-claude-cli-inference-backend-design.md`.
Run 2026-06-23 against Claude Code CLI **v2.1.177**, model `sonnet`.

## Decision: ✅ PASS — proceed with the design as written.

The CLI behaves as a pure text generator: it emits clean parseable text, it
reliably respects the `Prompted` fence format, and `--allowedTools ""` genuinely
disables its own tool execution.

## Working invocation

```
claude -p --output-format stream-json --verbose --allowedTools "" --model <model> \
  --system-prompt "<neutral generator instruction>" \
  --setting-sources project --strict-mcp-config --no-session-persistence
```

Prompt delivered on stdin. Exit code 0 on success.

**Flag rationale** (hardened after the initial spike, all verified to preserve
subscription auth):

- `--system-prompt` **replaces** the "you are Claude Code" harness prompt, so it
  can't compete with the Prompted tool preamble (retires the harness-prompt risk).
- `--setting-sources project` stops loading the **user** settings where the
  `SessionStart` hook lives — eliminating the context pollution noted below.
- `--strict-mcp-config` + `--no-session-persistence` skip MCP discovery and
  disk writes (stateless per turn) to cut per-turn overhead.
- **Do NOT use `--bare`.** It strips hooks/auto-memory/etc. in one flag, but its
  help states auth is *"strictly ANTHROPIC_API_KEY or apiKeyHelper … OAuth and
  keychain are never read"* — i.e. it disables the subscription auth that is the
  entire reason for this backend. The flags above achieve "bare-ish" behavior
  while keeping the subscription.

## Real captured event lines (fixtures source of truth)

**Assistant text event** (text lives at `message.content[].text` for blocks with
`type == "text"`):

```json
{"type":"assistant","message":{"model":"claude-sonnet-4-6","id":"msg_011bm9LmuX6XLQh1ZgGBMYFP","type":"message","role":"assistant","content":[{"type":"text","text":"hello world"}],"stop_reason":null,"stop_sequence":null},"session_id":"0ed91aae-..."}
```

**Terminal result event** (`stop_reason` is top-level on the result object):

```json
{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","num_turns":1,"result":"hello world","permission_denials":[],"session_id":"0ed91aae-..."}
```

The plan's Task 1 fixtures are structurally identical simplifications of these and
need **no change** — the JSON paths (`message.content[].text`, result
`stop_reason`/`subtype`) match the real output.

## Truncation signal

Normal completion: result `stop_reason == "end_turn"`. Truncation would be
`stop_reason == "max_tokens"` (the parser maps that → `StopReason::Length`).
`subtype == "error_max_turns"` also indicates a non-clean stop.

## Tool-disabling confirmed

With `--allowedTools ""` and a prompt instructing the `tool_call` fence, the model
emitted the fenced block **as text** and made **zero** `tool_use` calls
(`num_turns: 1`, `permission_denials: []`). The fenced output was byte-correct:

```
Reading `a.txt` now.

```tool_call
{"name":"read_file","arguments":{"path":"a.txt"}}
```
```

## Risks / notes for implementation

1. **SessionStart hooks fire in the nested invocation.** ✅ **RESOLVED** by
   `--setting-sources project` (see Working invocation above) — verified the
   `hook_started`/`hook_response` events disappear from the stream while
   subscription auth still works. Originally: the user's `SessionStart:startup`
   hook injected the entire `using-superpowers` skill into the generator's
   context (a large `{"type":"system","subtype":"hook_response",...}` line) and
   leaked into `thinking`. Caveat: project-local hooks *in the agent's workspace*
   would still load — only `--bare` loads nothing, and that breaks auth.
2. **`thinking` content blocks appear** before the text block. The parser must
   extract only `type == "text"` blocks (it does) — thinking is ignored.
3. **Other stdout event types seen, all ignored** by the parser's catch-all arm:
   `system` (`hook_started`, `hook_response`, `init`), `rate_limit_event`.
4. **Latency:** ~3.5 s for a trivial prompt (cold prompt-cache). Acceptable for
   a local dev backend; each turn is a fresh process so there is no warm reuse.
5. **Rate limits:** the run reported `rate_limit_event` with `five_hour` window —
   the subscription cap, as expected. Heavy loop use will hit it.

## Follow-ups / known limitations

Open items deferred out of the initial backend. None block the local-dev use the
design scoped for. Last reconciled 2026-06-23 after the branch merged to `main`.

### Open

**P1 — reliability (cross-cutting, needs its own spec)**

- [ ] **Add a per-turn timeout around model-stream consumption in `AgentLoop`**
  (`agent-core/src/loop_.rs:54-58` — bare `while let Some(item) = stream.next().await`;
  `tool_timeout` wraps only tool execution at ~line 167). A hung backend blocks the
  turn indefinitely. **Pre-existing** — affects the `OpenAiCompatClient`/SGLang path
  identically and lives in the core loop that's been kept untouched, so it needs a
  brainstorm→spec cycle. `ClaudeCliClient`'s `kill_on_drop(true)` already kills the
  subprocess once the stream is dropped, so a loop-level deadline would make that
  fire on a stall.

**P2 — `claude-cli` robustness (before sustained/automated use)**

- [ ] **Rate-limit strategy for the 5-hour subscription cap** (risk #5). No backoff
  today; detect `rate_limit_event` / surface a typed `ModelError` and back off before
  driving sustained loops.
- [ ] **Pin the subprocess CWD** to a known-empty scratch dir via
  `Command::current_dir()` (`claude_cli.rs` `stream`). `--setting-sources project`
  still loads project-local hooks from the launch dir; pinning fully isolates the
  generator. Small.

**P3 — monitoring / low priority**

- [ ] **Guard `BARE_SYSTEM_PROMPT` acceptance** (`claude_cli.rs`). If CLI guardrails
  ever reject the self-description prompt, the backend silently breaks. Optional: an
  `#[ignore]`-gated integration test against the real CLI.

### Resolved (kept for context)

- [x] Operator docs for `--backend` / `--claude-binary` — `agent/docs/RUNNING.md` §1, `cloud/RUNNING.md` §2.
- [x] SessionStart-hook context pollution (risk #1) — `--setting-sources project`.
- [x] ETXTBSY parallel-test flake — `serial_test` `#[serial]` on the process-spawning tests.
- [x] No flag-forwarding test — `forwards_bare_generator_flags` in `claude_cli.rs`.

### Accepted (won't-fix)

- Swallowed `stdin.write_all` result — benign; the real error surfaces via non-zero exit + stderr.
- `stderr_task.await.unwrap_or_default()` swallowing `JoinError` — harmless resilience.
