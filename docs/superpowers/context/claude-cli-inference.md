# Claude CLI as inference backend — Phase 0 spike notes

Spike for `docs/superpowers/specs/2026-06-23-claude-cli-inference-backend-design.md`.
Run 2026-06-23 against Claude Code CLI **v2.1.177**, model `sonnet`.

## Decision: ✅ PASS — proceed with the design as written.

The CLI behaves as a pure text generator: it emits clean parseable text, it
reliably respects the `Prompted` fence format, and `--allowedTools ""` genuinely
disables its own tool execution.

## Working invocation

```
claude -p --output-format stream-json --verbose --allowedTools "" --model sonnet
```

Prompt delivered on stdin. Exit code 0 on success.

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

1. **SessionStart hooks fire in the nested invocation.** The user's
   `SessionStart:startup` hook injects the entire `using-superpowers` skill into
   the generator's context (visible as a large `{"type":"system","subtype":"hook_response",...}`
   stdout line, and it leaks into the model's `thinking`). Harmless to the parser
   (all `type":"system"` lines are ignored) and it still produced correct output,
   but it is context pollution and burns tokens. Mitigation deferred; revisit with
   `--settings` / hook-suppression flags if it causes misbehavior.
2. **`thinking` content blocks appear** before the text block. The parser must
   extract only `type == "text"` blocks (it does) — thinking is ignored.
3. **Other stdout event types seen, all ignored** by the parser's catch-all arm:
   `system` (`hook_started`, `hook_response`, `init`), `rate_limit_event`.
4. **Latency:** ~3.5 s for a trivial prompt (cold prompt-cache). Acceptable for
   a local dev backend; each turn is a fresh process so there is no warm reuse.
5. **Rate limits:** the run reported `rate_limit_event` with `five_hour` window —
   the subscription cap, as expected. Heavy loop use will hit it.

## Follow-ups / known limitations

Captured from the final whole-branch review (opus, 2026-06-23). None block the
local-dev use the design scoped for; all are out of scope for the initial
backend and tracked here so they survive the merge.

1. **Operator docs for the new flags.** Nothing user-facing documents
   `--backend {openai|claude-cli}` or `--claude-binary` (e.g. in a `RUNNING.md`).
   A short stanza should cover: the authenticated-CLI prerequisite, the
   `sonnet`/`opus` model values, and the rate-limit caveat (risk #5).
2. **Production risks to track before non-dev use** (already noted as risks #1
   and #5 above): the SessionStart-hook context pollution (every nested turn
   ingests the user's `using-superpowers` injection, burning tokens and leaking
   into `thinking`) and the 5-hour subscription rate cap. Investigate
   `--settings` / hook-suppression and a backoff/limit strategy before driving
   sustained loops.
3. **`AgentLoop` has no timeout around model-stream consumption**
   (`agent-core/src/loop_.rs:54-58` — bare `while let Some(item) = stream.next().await`;
   `tool_timeout` wraps only tool execution at ~line 167). A hung backend blocks
   the turn indefinitely. **Pre-existing** — it affects the `OpenAiCompatClient`
   /SGLang path identically and lives in the FIXED `AgentLoop`, so it was out of
   scope for this branch. `ClaudeCliClient`'s `kill_on_drop(true)` already cleans
   up the subprocess *if* the stream is dropped; adding a per-turn deadline in the
   loop would make that path actually trigger on a stall. Fix belongs to a
   loop-level change, not this backend.
