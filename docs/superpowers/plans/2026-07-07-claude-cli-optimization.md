# Claude CLI Backend Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Optimize `ClaudeCliClient` (session reuse via prefix-matching, partial-message streaming, thinking→`Chunk::Reasoning`, `--effort`/`--fallback-model` knobs), grounded by a research phase whose findings ship as an OKF bundle at `docs/okf/claude-cli-headless/`.

**Architecture:** The agent loop stays in agent-core (approved spec, Option B). `ClaudeCliClient` gains internal session state (`Mutex<Option<SessionState>>`) and a three-state spawn plan: fresh-ephemeral → fresh-persisted → resume-with-suffix. A stateful `EventParser` replaces the free `parse_event_line`, adding `stream_event` delta parsing, thinking blocks, `session_id` capture, and delta/whole-message dedup. Config knobs ride `RuntimeConfig` with serde defaults.

**Tech Stack:** Rust (tokio, async-stream, serde_json), Claude Code CLI 2.1.195 (`stream-json`), OKF bundle validated by `scripts/okf_check.py`.

**Spec:** `docs/superpowers/specs/2026-07-07-claude-cli-optimization-design.md`

## Global Constraints

- Rust work happens in the `agent/` workspace: `cd /home/kalen/rust-agent-runtime/agent`.
- Conventional commits: `type(scope): summary`.
- No `ModelClient` trait changes. claude-cli stays prompted-only (`normalized()`/`validate()` untouched).
- New `RuntimeConfig` fields must be `#[serde(default)]`-ed so existing configs parse unchanged.
- OKF checker rules (`scripts/okf_check.py`): frontmatter is flat `key: value` / `key: [a, b]` inline lists only; `type` ∈ {Source, Practice, Lifecycle Phase, Perspective, Comparison}; every `Source` needs non-empty `resource:`; nodes under `practices/`/`comparisons/` need a `# Citations` section with ≥1 `/sources/...` link; every non-root dir `index.md` lists every non-reserved sibling; bundle-root `index.md` frontmatter may declare only `okf_version`; `log.md` has none.
- The `capabilities/` directory is not in the checker's citation-enforced set — its nodes still use `type: Practice` (only allowed vocabulary) and include `# Citations` anyway for consistency.
- Probe commands in Task 1 make real API calls via the local `claude` subscription — keep prompts tiny.

---

### Task 1: Probe the CLI + fetch docs (research captures)

**Files:**
- Create: `docs/okf/claude-cli-headless/sources/headless-print-mode.md`
- Create: `docs/okf/claude-cli-headless/sources/cli-reference.md`
- Create: `docs/okf/claude-cli-headless/sources/probe-stream-json-2-1-195.md`
- Create: `docs/okf/claude-cli-headless/sources/probe-resume-2-1-195.md`
- Create: `docs/okf/claude-cli-headless/sources/probe-model-knobs-2-1-195.md`
- Create: `docs/okf/claude-cli-headless/sources/index.md`

**Interfaces:**
- Consumes: local `claude` binary (2.1.195), scratchpad dir for raw captures.
- Produces: verified stream-json line shapes (init / stream_event deltas / assistant / result), the resume round-trip recipe, the `--effort` allowed-value list, cache-read evidence. Tasks 2–6 cite these files; Task 3's test literals and Task 6's `EFFORT_LEVELS` const come from here.

- [ ] **Step 1: Capture a plain streaming call (deltas + thinking shapes)**

Run (note the scratchpad dir; keep it for all steps):

```bash
SCRATCH=/tmp/claude-1000/-home-kalen-rust-agent-runtime/008f3191-660c-4f98-8bbd-5d0c4e6bbec1/scratchpad
mkdir -p "$SCRATCH/probes"
claude -p --output-format stream-json --verbose --include-partial-messages \
  --allowedTools "" --model sonnet \
  --system-prompt "You are a text generator. Follow the instructions in the message exactly." \
  --setting-sources project --strict-mcp-config --no-session-persistence \
  <<< "Say exactly: hello" > "$SCRATCH/probes/plain.jsonl" 2>"$SCRATCH/probes/plain.err"
wc -l "$SCRATCH/probes/plain.jsonl"; head -c 2000 "$SCRATCH/probes/plain.jsonl"
```

Expected: a `{"type":"system","subtype":"init","session_id":"..."}` line, one or more `{"type":"stream_event",...}` lines whose `event.type == "content_block_delta"` with `delta.type == "text_delta"`, a whole `{"type":"assistant",...}` message, and a final `{"type":"result",...}` with `usage`. Record the **verbatim** delta line and note whether the whole assistant message repeats the delta text (this pins the dedup rule).

- [ ] **Step 2: Capture thinking output**

```bash
claude -p --output-format stream-json --verbose --include-partial-messages \
  --allowedTools "" --model opus --effort high \
  --setting-sources project --strict-mcp-config --no-session-persistence \
  <<< "What is 17*23? Think it through, then answer with just the number." \
  > "$SCRATCH/probes/thinking.jsonl" 2>&1
grep -o '"type":"thinking_delta"' "$SCRATCH/probes/thinking.jsonl" | head -1
grep -o '"type":"thinking"' "$SCRATCH/probes/thinking.jsonl" | head -1
```

Expected: `thinking_delta` stream events and/or a `thinking` content block in the whole assistant message. Record verbatim examples of both shapes (field name carrying the text — expected `delta.thinking` / block `thinking`). If neither appears, retry with `--model sonnet`; if still absent, record "no thinking emitted at this tier/config" — Task 3 keeps the parser arms anyway (shape from Anthropic SSE docs).

- [ ] **Step 3: Resume round-trip + cache evidence**

```bash
cd "$SCRATCH/probes"
claude -p --output-format stream-json --verbose --allowedTools "" --model sonnet \
  --setting-sources project --strict-mcp-config \
  <<< "The codeword is umbrella. Reply OK." > resume1.jsonl 2>&1
SID=$(grep -o '"session_id":"[^"]*"' resume1.jsonl | head -1 | cut -d'"' -f4)
echo "SID=$SID"
claude -p --resume "$SID" --output-format stream-json --verbose --allowedTools "" --model sonnet \
  --setting-sources project --strict-mcp-config \
  <<< "What is the codeword? Answer with one word." > resume2.jsonl 2>&1
grep -io umbrella resume2.jsonl | head -1
grep -o '"cache_read_input_tokens":[0-9]*' resume2.jsonl
grep -o '"session_id":"[^"]*"' resume2.jsonl | head -1
```

Expected: `umbrella` recalled (proves state carried); `cache_read_input_tokens > 0` on the resumed call (caching economics evidence); note whether the resumed call's init `session_id` equals `$SID` or is new (pins whether the client must re-capture the id from each init — Task 5 re-captures regardless). Also record where the session file landed (`ls ~/.claude/projects/*/$SID.jsonl 2>/dev/null` or note the actual location) and what happens on a bogus resume: `claude -p --resume 00000000-0000-0000-0000-000000000000 ... <<< "hi"` — record exit code and stderr text.

- [ ] **Step 4: Pin `--effort` values and `--fallback-model` acceptance**

```bash
claude -p --effort banana --output-format json --no-session-persistence <<< "hi" 2>&1 | head -5
claude -p --fallback-model sonnet --output-format json --no-session-persistence \
  --allowedTools "" --model opus --setting-sources project --strict-mcp-config <<< "Say hi" 2>&1 | tail -3
```

Expected: the first command errors with the allowed effort values (record the exact list — Task 6's `EFFORT_LEVELS` const copies it verbatim); the second succeeds (flag accepted in print mode).

- [ ] **Step 5: Fetch the official docs**

WebFetch these (if a URL 404s, WebSearch "claude code headless mode docs" for the current location and note the URL you actually used):

- `https://code.claude.com/docs/en/headless` — prompt: "Extract headless/print mode usage: stream-json output format, --include-partial-messages, session resume flags, --resume/--session-id semantics"
- `https://code.claude.com/docs/en/cli-reference` — prompt: "Extract flag semantics for --resume, --session-id, --fork-session, --include-partial-messages, --effort, --fallback-model, --no-session-persistence, --setting-sources, --strict-mcp-config"

Save the extracted notes (not raw HTML) in the scratchpad.

- [ ] **Step 6: Write the six `sources/` files**

Every source file uses this frontmatter shape (inline values only, both keys required):

```markdown
---
type: Source
resource: <URL fetched, or for probes: https://github.com/anthropics/claude-code>
---
```

- `headless-print-mode.md`, `cli-reference.md`: distilled notes from Step 5, with the `resource:` URL actually fetched.
- `probe-stream-json-2-1-195.md`: verbatim captured lines from Steps 1–2 (init, text_delta, thinking_delta, whole assistant, result) in fenced blocks, plus the dedup observation. State binary version 2.1.195 in the body.
- `probe-resume-2-1-195.md`: the Step 3 recipe, codeword-recall result, `cache_read_input_tokens` evidence, same-vs-new session id observation, session file location, bogus-resume failure shape.
- `probe-model-knobs-2-1-195.md`: the exact `--effort` allowed-value list and `--fallback-model` acceptance from Step 4.
- `sources/index.md`: `index.md` is a reserved name, so it carries **no frontmatter**; body is a bullet list linking every sibling: `- [headless-print-mode](headless-print-mode.md) — ...` (one line per file, all five).

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add docs/okf/claude-cli-headless/sources/
git commit -m "docs(okf): claude-cli-headless sources (docs + 2.1.195 probes)"
```

---

### Task 2: Author the OKF bundle

**Files:**
- Create: `docs/okf/claude-cli-headless/index.md`, `docs/okf/claude-cli-headless/log.md`
- Create: `docs/okf/claude-cli-headless/capabilities/{index.md,session-resume.md,partial-message-streaming.md,thinking-output.md,model-knobs.md,caching-economics.md}`
- Create: `docs/okf/claude-cli-headless/practices/{index.md,delta-resume.md,prefix-invalidation.md,auth-preservation.md,stderr-drain.md,flag-pinning-tests.md}`
- Create: `docs/okf/claude-cli-headless/comparisons/{index.md,stateless-vs-session-resume.md,prompted-vs-native-tools.md}`

**Interfaces:**
- Consumes: Task 1 `sources/` files (every claim cites into `/sources/`).
- Produces: `python3 scripts/okf_check.py docs/okf/claude-cli-headless` → `OK`. The bundle is the durable rationale record Tasks 3–6 implement against.

- [ ] **Step 1: Write root `index.md` and `log.md`**

`index.md` (frontmatter may declare **only** `okf_version` — match the agent-sdlc root):

```markdown
---
okf_version: 0.1
---

# Claude CLI Headless — Knowledge Bundle

How this repo drives the Claude Code CLI (2.1.195) as a `ModelClient` backend:
verified stream-json shapes, session-resume economics, and the practices the
`agent-model` claude_cli client implements.

- [sources/](sources/index.md) — fetched docs + local probe captures
- [capabilities/](capabilities/index.md) — what the CLI's headless surface offers
- [practices/](practices/index.md) — how this repo uses it
- [comparisons/](comparisons/index.md) — declined alternatives and why
```

`log.md` (no frontmatter):

```markdown
# Verification log

- 2026-07-07: bundle created; all probe claims verified against claude 2.1.195
  on this machine. Re-verify probes after any CLI major-version bump.
```

- [ ] **Step 2: Write the five `capabilities/` nodes**

Frontmatter for every node in this dir:

```markdown
---
type: Practice
tags: [claude-cli, capability]
---
```

Each file: `# <Title>`, 2–4 paragraphs sourced **only** from Task 1 findings, then `# Citations` with numbered `/sources/...` links (body claims may use `[1]` markers; every marker needs a matching numbered entry). Content requirements:

- `session-resume.md`: `--resume <id>` semantics, id capture from `system/init`, persistence requirement (`--no-session-persistence` forfeits resumability), session file location, bogus-resume failure shape.
- `partial-message-streaming.md`: `--include-partial-messages` → `stream_event`/`content_block_delta` lines, verbatim example, the duplicate-whole-message observation.
- `thinking-output.md`: thinking_delta / thinking block shapes as captured (or the documented shape + a note that the probe didn't elicit thinking, if that's what Task 1 found).
- `model-knobs.md`: `--effort` allowed values (exact probed list), `--fallback-model` semantics, `--betas` existence (unexercised).
- `caching-economics.md`: the `cache_read_input_tokens` evidence from the resume probe; why resumed calls reprocess only the suffix.

- [ ] **Step 3: Write the five `practices/` nodes**

Same frontmatter with `tags: [claude-cli, practice]`. `delta-resume.md` in full (adapt numbers/citations to actual sources):

```markdown
---
type: Practice
tags: [claude-cli, practice]
---

# Delta resume (persist on second use)

The client spawns a fresh CLI process per model call, so "session reuse" means
resuming the CLI's persisted conversation instead of re-piping the whole
transcript. State machine:

1. **First call** on a transcript: `--no-session-persistence`, full transcript
   piped. One-shot workloads (compaction, evals) never write session files.
2. **First append-only extension**: full transcript again, persistence ON;
   the `system/init` event's `session_id` is recorded [1].
3. **Later extensions**: `--resume <id>` with only the *suffix* piped
   (assistant turns skipped — the CLI session already holds its own replies).
   Resumed calls show `cache_read_input_tokens > 0` [2].

Cost shape: one extra full send per session (step 2) buys suffix-only sends for
every later round; no disk writes for single-shot callers. Any non-extension
(history rewritten by curation/compaction) resets to step 1 automatically —
see [prefix-invalidation](prefix-invalidation.md).

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
2. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
```

Content requirements for the rest:

- `prefix-invalidation.md`: fingerprints (role+name+content+reasoning hash per message), strict-extension check, why prefix-matching decouples the client from the context manager, error → state reset → the loop's retry lands on a fresh session.
- `auth-preservation.md`: the `--bare` footgun (forces API-key auth, defeats subscription piggybacking), `AGENT_API_KEY` env removal, `--setting-sources project` to skip SessionStart hooks.
- `stderr-drain.md`: the concurrent stderr drain (~64 KiB pipe-buffer deadlock), stdin fed on a separate task.
- `flag-pinning-tests.md`: the fake-CLI proc-test pattern that fails the stream when a required flag goes missing, and why each pinned flag is load-bearing.

- [ ] **Step 4: Write the two `comparisons/` nodes**

Frontmatter `type: Comparison`, `tags: [claude-cli]`. Content requirements (each with `# Citations` into `/sources/`):

- `stateless-vs-session-resume.md`: full-send-every-call vs delta resume; token/latency cost per tool round; the rewrite-fallback cost (two full sends after a curation event); when stateless is still right (`claude_session_reuse: false`).
- `prompted-vs-native-tools.md`: record the **declined** design options from the spec — native tool_use requires MCP bridging (CLI owns the inner loop, context manager idles) or full delegation (bypasses agent-sandbox/policy, breaks backend parity); this repo keeps the prompted protocol with the loop in agent-core. Cite the spec path in prose (`docs/superpowers/specs/2026-07-07-claude-cli-optimization-design.md` — as plain text, not a markdown link, since intra-bundle links must resolve inside the bundle).

- [ ] **Step 5: Write the three directory `index.md` files**

No frontmatter; each lists **every** non-reserved sibling as a markdown link with a one-line hook (checker rule 8).

- [ ] **Step 6: Run the checker until green**

```bash
python3 /home/kalen/rust-agent-runtime/scripts/okf_check.py /home/kalen/rust-agent-runtime/docs/okf/claude-cli-headless
```

Expected: `OK`. Fix any listed error (broken link, missing citation, unlisted node) and re-run.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add docs/okf/claude-cli-headless/
git commit -m "docs(okf): claude-cli-headless bundle (capabilities, practices, comparisons)"
```

---

### Task 3: Stateful `EventParser` (deltas, thinking, session id, dedup)

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs` (replace `parse_event_line` + its tests)

**Interfaces:**
- Consumes: `Chunk`, `StopReason`, `ModelError` from `crate` (existing).
- Produces: `pub(crate) struct EventParser { pub(crate) session_id: Option<String>, .. }` with `pub(crate) fn new() -> Self` and `pub(crate) fn parse_line(&mut self, line: &str) -> Result<Vec<Chunk>, ModelError>`. Task 5's stream loop constructs one per spawn and reads `.session_id` after EOF.

- [ ] **Step 1: Write the failing tests**

Replace the whole `mod tests` block in `claude_cli.rs` with the version below. **First substitute the fixture literals with the verbatim captured lines from `docs/okf/claude-cli-headless/sources/probe-stream-json-2-1-195.md`** — the shapes below are the documented defaults and only stand in until then; if a captured field name differs (e.g. delta text under a different key), the test literal wins and the implementation follows it.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    // Fixture lines: verbatim captures from
    // docs/okf/claude-cli-headless/sources/probe-stream-json-2-1-195.md (claude 2.1.195).
    const INIT_LINE: &str = r#"{"type":"system","subtype":"init","session_id":"sess-abc"}"#;
    const TEXT_DELTA_LINE: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hel"}},"session_id":"sess-abc"}"#;
    const THINKING_DELTA_LINE: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}},"session_id":"sess-abc"}"#;
    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello world"}]},"session_id":"sess-abc"}"#;
    const ASSISTANT_THINKING_LINE: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"a plan"},{"type":"text","text":"hello world"}]},"session_id":"sess-abc"}"#;
    const RESULT_LINE: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"hello world","session_id":"sess-abc"}"#;

    #[test]
    fn init_line_captures_session_id_and_emits_nothing() {
        let mut p = EventParser::new();
        assert!(p.parse_line(INIT_LINE).unwrap().is_empty());
        assert_eq!(p.session_id.as_deref(), Some("sess-abc"));
    }

    #[test]
    fn text_delta_emits_text_chunk() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(TEXT_DELTA_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Text(t)] if t == "hel"));
    }

    #[test]
    fn thinking_delta_emits_reasoning_chunk() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(THINKING_DELTA_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Reasoning(t)] if t == "hmm"));
    }

    #[test]
    fn whole_assistant_message_is_skipped_after_deltas() {
        let mut p = EventParser::new();
        p.parse_line(TEXT_DELTA_LINE).unwrap();
        assert!(p.parse_line(ASSISTANT_LINE).unwrap().is_empty());
    }

    #[test]
    fn whole_assistant_message_emits_when_no_deltas_seen() {
        // Back-compat: a CLI that ignores --include-partial-messages still streams.
        let mut p = EventParser::new();
        let chunks = p.parse_line(ASSISTANT_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Text(t)] if t == "hello world"));
    }

    #[test]
    fn whole_assistant_thinking_block_emits_reasoning_when_no_deltas() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(ASSISTANT_THINKING_LINE).unwrap();
        assert!(matches!(&chunks[0], Chunk::Reasoning(t) if t == "a plan"));
        assert!(matches!(&chunks[1], Chunk::Text(t) if t == "hello world"));
    }

    #[test]
    fn result_event_emits_done_stop() {
        let mut p = EventParser::new();
        let chunks = p.parse_line(RESULT_LINE).unwrap();
        assert!(matches!(chunks.as_slice(), [Chunk::Done(StopReason::Stop)]));
    }

    #[test]
    fn result_event_carries_usage_and_cost() {
        let line = r#"{"type":"result","subtype":"success","total_cost_usd":0.0421,"usage":{"input_tokens":1200,"output_tokens":345}}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(chunks.iter().any(|c| matches!(c,
            Chunk::Usage { prompt_tokens: 1200, completion_tokens: 345,
                           cost_usd: Some(c), .. } if (*c - 0.0421).abs() < 1e-9)));
        assert!(matches!(chunks.last(), Some(Chunk::Done(StopReason::Stop))));
    }

    #[test]
    fn result_event_folds_cache_tokens_into_prompt() {
        let line = r#"{"type":"result","subtype":"success","usage":{"input_tokens":1000,"cache_read_input_tokens":4000,"cache_creation_input_tokens":500,"output_tokens":42}}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(chunks.iter().any(|c| matches!(
            c,
            Chunk::Usage { prompt_tokens: 5500, completion_tokens: 42, cached_tokens: Some(4000), .. }
        )));
    }

    #[test]
    fn max_turns_result_maps_to_length() {
        let line = r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
        let chunks = EventParser::new().parse_line(line).unwrap();
        assert!(matches!(chunks.last(), Some(Chunk::Done(StopReason::Length))));
    }

    #[test]
    fn blank_line_yields_nothing() {
        assert!(EventParser::new().parse_line("  ").unwrap().is_empty());
    }

    #[test]
    fn non_json_line_is_decode_error() {
        assert!(matches!(
            EventParser::new().parse_line("not json"),
            Err(ModelError::Decode(_))
        ));
    }

    // --- render_transcript tests: keep the existing six verbatim ---
    // renders_roles_with_headers, tool_message_includes_tool_name_in_header,
    // assistant_message_rendered, preserved_reasoning_renders_as_think_block_before_content,
    // no_reasoning_renders_content_only
    // (copy them unchanged from the current file)
}
```

Note: `EventParser::new().parse_line(...)` on a temporary needs a binding in some cases — write `let mut p = EventParser::new();` where the compiler demands it.

- [ ] **Step 2: Run to verify failure**

```bash
cd /home/kalen/rust-agent-runtime/agent && cargo test -p agent-model claude_cli
```

Expected: FAIL — `EventParser` not found.

- [ ] **Step 3: Implement `EventParser`**

Replace `parse_event_line` with:

```rust
/// Stateful stream-json line parser: one instance per CLI spawn. Tracks the
/// init event's session_id (for resume) and whether stream_event deltas were
/// seen (whole assistant messages then duplicate the deltas and are skipped).
pub(crate) struct EventParser {
    pub(crate) session_id: Option<String>,
    saw_stream_deltas: bool,
}

impl EventParser {
    pub(crate) fn new() -> Self {
        Self {
            session_id: None,
            saw_stream_deltas: false,
        }
    }

    pub(crate) fn parse_line(&mut self, line: &str) -> Result<Vec<Chunk>, ModelError> {
        let line = line.trim();
        if line.is_empty() {
            return Ok(vec![]);
        }
        let v: Value =
            serde_json::from_str(line).map_err(|e| ModelError::Decode(e.to_string()))?;
        let mut out = Vec::new();
        match v["type"].as_str() {
            Some("system") => {
                if v["subtype"] == "init" {
                    if let Some(id) = v["session_id"].as_str() {
                        self.session_id = Some(id.to_string());
                    }
                }
            }
            Some("stream_event") => {
                let ev = &v["event"];
                if ev["type"] == "content_block_delta" {
                    match ev["delta"]["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(t) = ev["delta"]["text"].as_str() {
                                if !t.is_empty() {
                                    self.saw_stream_deltas = true;
                                    out.push(Chunk::Text(t.to_string()));
                                }
                            }
                        }
                        Some("thinking_delta") => {
                            if let Some(t) = ev["delta"]["thinking"].as_str() {
                                if !t.is_empty() {
                                    self.saw_stream_deltas = true;
                                    out.push(Chunk::Reasoning(t.to_string()));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("assistant") => {
                // With --include-partial-messages the whole message repeats what
                // the deltas already streamed — emit only if no deltas were seen
                // (back-compat with a CLI that ignores the flag).
                if !self.saw_stream_deltas {
                    if let Some(blocks) = v["message"]["content"].as_array() {
                        for b in blocks {
                            match b["type"].as_str() {
                                Some("text") => {
                                    if let Some(t) = b["text"].as_str() {
                                        if !t.is_empty() {
                                            out.push(Chunk::Text(t.to_string()));
                                        }
                                    }
                                }
                                Some("thinking") => {
                                    if let Some(t) = b["thinking"].as_str() {
                                        if !t.is_empty() {
                                            out.push(Chunk::Reasoning(t.to_string()));
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            Some("result") => {
                if let Some(u) = v.get("usage").and_then(Value::as_object) {
                    let field = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    let cache_read = field("cache_read_input_tokens");
                    // Fold cache tokens into prompt_tokens so it reflects the
                    // effective context size; cached_tokens still surfaces the
                    // cache-read portion separately.
                    out.push(Chunk::Usage {
                        prompt_tokens: (field("input_tokens")
                            + cache_read
                            + field("cache_creation_input_tokens"))
                            as u32,
                        completion_tokens: field("output_tokens") as u32,
                        reasoning_tokens: None,
                        cached_tokens: if cache_read > 0 {
                            Some(cache_read as u32)
                        } else {
                            None
                        },
                        cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
                    });
                }
                let truncated = v["subtype"].as_str() == Some("error_max_turns")
                    || v["stop_reason"].as_str() == Some("max_tokens");
                out.push(Chunk::Done(if truncated {
                    StopReason::Length
                } else {
                    StopReason::Stop
                }));
            }
            _ => {} // user echoes etc. — nothing to emit.
        }
        Ok(out)
    }
}
```

Update the `stream()` body to use it (minimal interim wiring; Task 5 rewrites this section again):

```rust
let stream = async_stream::stream! {
    let mut parser = EventParser::new();
    let mut lines = BufReader::new(stdout).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => match parser.parse_line(&line) {
                Ok(chunks) => {
                    for c in chunks {
                        yield Ok(c);
                    }
                }
                Err(e) => {
                    yield Err(e);
                    return;
                }
            },
            Ok(None) => break,
            Err(e) => {
                yield Err(ModelError::Stream(e.to_string()));
                return;
            }
        }
    }
    // ... child.wait() block unchanged ...
};
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent-model claude_cli
```

Expected: PASS (all parser tests + proc tests + transcript tests).

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs
git commit -m "feat(model): stateful claude-cli event parser (deltas, thinking, session id, dedup)"
```

---

### Task 4: `ClaudeCliOptions` + flag wiring (effort, fallback, partial messages)

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs`
- Modify: `agent/crates/agent-model/src/lib.rs` (re-export)

**Interfaces:**
- Consumes: `EventParser` (Task 3).
- Produces: `pub struct ClaudeCliOptions { pub session_reuse: bool, pub effort: Option<String>, pub fallback_model: Option<String> }` (derives `Debug, Clone, Default`); `ClaudeCliClient::new(binary, model)` (unchanged semantics, `Default` options = today's behavior) and `ClaudeCliClient::with_options(binary, model, opts)`. Re-exported from `agent_model`. Task 6 constructs `ClaudeCliOptions` from config; Task 5 reads `opts.session_reuse`.

- [ ] **Step 1: Write the failing proc tests**

In `mod proc_tests`, update `forwards_bare_generator_flags` to pin the new always-on flag, and add the knob test:

```rust
#[tokio::test]
#[serial]
async fn forwards_bare_generator_flags() {
    // Fails unless every load-bearing bare-generator flag is present.
    let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
        for f in --system-prompt --setting-sources --strict-mcp-config --no-session-persistence --allowedTools --include-partial-messages; do\n\
          case \" $* \" in *\" $f \"*) ;; *) echo \"missing $f\" >&2; exit 3;; esac\n\
        done\n\
        echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"session_id\":\"t\"}'\n\
        echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"t\"}'\n";
    let fake = write_fake(script);
    let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
    let mut stream = client.stream(req()).await.unwrap();
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        if let Chunk::Text(t) = item.unwrap() {
            text.push_str(&t);
        }
    }
    assert_eq!(text, "ok");
}

#[tokio::test]
#[serial]
async fn forwards_effort_and_fallback_model_flags() {
    let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
        case \" $* \" in *\" --effort high \"*) ;; *) echo 'missing --effort' >&2; exit 3;; esac\n\
        case \" $* \" in *\" --fallback-model sonnet \"*) ;; *) echo 'missing --fallback-model' >&2; exit 3;; esac\n\
        echo '{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]},\"session_id\":\"t\"}'\n\
        echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"t\"}'\n";
    let fake = write_fake(script);
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "opus",
        ClaudeCliOptions {
            effort: Some("high".into()),
            fallback_model: Some("sonnet".into()),
            ..Default::default()
        },
    );
    let mut stream = client.stream(req()).await.unwrap();
    let mut text = String::new();
    while let Some(item) = stream.next().await {
        if let Chunk::Text(t) = item.unwrap() {
            text.push_str(&t);
        }
    }
    assert_eq!(text, "ok");
}

#[tokio::test]
#[serial]
async fn default_options_omit_knob_flags() {
    let script = "#!/usr/bin/env bash\ncat >/dev/null\n\
        case \" $* \" in *\" --effort \"*|*\" --fallback-model \"*) echo 'unexpected knob flag' >&2; exit 3;; esac\n\
        echo '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false}'\n";
    let fake = write_fake(script);
    let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet");
    let mut stream = client.stream(req()).await.unwrap();
    let mut done = None;
    while let Some(item) = stream.next().await {
        if let Chunk::Done(r) = item.unwrap() {
            done = Some(r);
        }
    }
    assert_eq!(done, Some(StopReason::Stop));
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p agent-model claude_cli
```

Expected: FAIL — `ClaudeCliOptions` / `with_options` not found (and `forwards_bare_generator_flags` fails on missing `--include-partial-messages`).

- [ ] **Step 3: Implement options + command builder**

```rust
/// Behavior knobs for the claude-cli backend. `Default` reproduces the
/// pre-optimization behavior exactly (stateless, no knob flags).
#[derive(Debug, Clone, Default)]
pub struct ClaudeCliOptions {
    /// Resume the CLI session across calls when the transcript extends
    /// append-only (delta resume). Off = stateless full send every call.
    pub session_reuse: bool,
    /// `--effort <level>`; validated upstream against the CLI's accepted set.
    pub effort: Option<String>,
    /// `--fallback-model <model>` when the primary is unavailable.
    pub fallback_model: Option<String>,
}

pub struct ClaudeCliClient {
    binary: String,
    model: String,
    opts: ClaudeCliOptions,
    state: std::sync::Arc<std::sync::Mutex<Option<SessionState>>>, // used from Task 5
}

impl ClaudeCliClient {
    pub fn new(binary: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_options(binary, model, ClaudeCliOptions::default())
    }

    pub fn with_options(
        binary: impl Into<String>,
        model: impl Into<String>,
        opts: ClaudeCliOptions,
    ) -> Self {
        Self {
            binary: binary.into(),
            model: model.into(),
            opts,
            state: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}
```

(For this task, add a placeholder-free minimal `SessionState` so the struct compiles — Task 5 fills the logic: `#[derive(Debug, Clone)] struct SessionState { session_id: Option<String>, persisted: bool, fingerprints: Vec<u64> }`.)

Extract the spawn into a builder used by `stream()` (plan handling arrives in Task 5; for now always ephemeral):

```rust
fn base_command(&self) -> Command {
    let mut cmd = Command::new(&self.binary);
    cmd.arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        // Token-level deltas instead of whole assistant messages.
        .arg("--include-partial-messages")
        .arg("--allowedTools")
        .arg("")
        .arg("--model")
        .arg(&self.model)
        // `--system-prompt` REPLACES the "you are Claude Code" harness prompt
        // (so it can't compete with the Prompted tool preamble on stdin).
        .arg("--system-prompt")
        .arg(BARE_SYSTEM_PROMPT)
        // Don't load the user's settings — that's where SessionStart hooks live.
        .arg("--setting-sources")
        .arg("project")
        .arg("--strict-mcp-config");
    if let Some(e) = &self.opts.effort {
        cmd.arg("--effort").arg(e);
    }
    if let Some(f) = &self.opts.fallback_model {
        cmd.arg("--fallback-model").arg(f);
    }
    // The CLI authenticates via its own subscription/OAuth — it must not
    // inherit the runtime's model API key.
    // NOTE: do NOT use `--bare` — it forces API-key auth and never reads
    // OAuth/keychain, defeating the subscription piggyback.
    cmd.env_remove("AGENT_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}
```

In `stream()`, replace the inline `Command::new(...)` chain with:

```rust
let mut cmd = self.base_command();
cmd.arg("--no-session-persistence"); // Task 5 makes this plan-dependent
let mut child = cmd
    .spawn()
    .map_err(|e| ModelError::Process(format!("spawn {}: {e}", self.binary)))?;
```

In `agent/crates/agent-model/src/lib.rs`, extend the existing claude_cli re-export to include `ClaudeCliOptions` (find the line re-exporting `ClaudeCliClient` and add the new type alongside it).

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent-model claude_cli
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs agent/crates/agent-model/src/lib.rs
git commit -m "feat(model): claude-cli options (effort, fallback-model) + partial-message streaming flag"
```

---

### Task 5: Session state machine (delta resume)

**Files:**
- Modify: `agent/crates/agent-model/src/claude_cli.rs`

**Interfaces:**
- Consumes: `ClaudeCliOptions.session_reuse` (Task 4), `EventParser.session_id` (Task 3), `Message`/`Role` from `crate`.
- Produces: internal only — `SessionState`, `SpawnPlan`, `fn plan_spawn`, `fn fingerprint`, `fn is_strict_extension`, suffix rendering. No public API change beyond behavior under `session_reuse: true`.

- [ ] **Step 1: Write the failing proc tests**

Add to `mod proc_tests` a multi-call fake that logs argv and stdin per invocation:

```rust
/// Fake CLI that records argv/stdin per call into `dir` and emits a canned
/// stream with session id "sess-<n>". `fail_call` (0 = never) exits 1 on that call.
fn write_recording_fake(dir: &std::path::Path, fail_call: u32) -> tempfile::TempPath {
    let d = dir.display();
    let script = format!(
        "#!/usr/bin/env bash\n\
         n=$(cat {d}/count 2>/dev/null || echo 0); n=$((n+1)); echo $n > {d}/count\n\
         printf '%s\\n' \"$*\" > {d}/argv.$n\n\
         cat > {d}/stdin.$n\n\
         if [ \"$n\" -eq \"{fail_call}\" ]; then echo boom >&2; exit 1; fi\n\
         echo '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"sess-'$n'\"}}'\n\
         echo '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"ok'$n'\"}}]}},\"session_id\":\"sess-'$n'\"}}'\n\
         echo '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"usage\":{{\"input_tokens\":1,\"output_tokens\":1}}}}'\n"
    );
    write_fake(&script)
}

async fn drain(client: &ClaudeCliClient, messages: Vec<Message>) -> Result<(), ModelError> {
    let mut stream = client
        .stream(CompletionRequest {
            messages,
            ..Default::default()
        })
        .await?;
    while let Some(item) = stream.next().await {
        item?;
    }
    Ok(())
}

fn read(dir: &std::path::Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

#[tokio::test]
#[serial]
async fn session_reuse_walks_ephemeral_persisted_resume() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 0);
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );

    let base = vec![Message::system("sys"), Message::user("u1")];
    drain(&client, base.clone()).await.unwrap();

    let mut ext1 = base.clone();
    ext1.push(Message::assistant("ok1", None));
    ext1.push(Message::tool("call_0", "read_file", "t1"));
    drain(&client, ext1.clone()).await.unwrap();

    let mut ext2 = ext1.clone();
    ext2.push(Message::assistant("ok2", None));
    ext2.push(Message::user("u2"));
    drain(&client, ext2).await.unwrap();

    // Call 1: ephemeral full send.
    let argv1 = read(dir.path(), "argv.1");
    assert!(argv1.contains("--no-session-persistence"), "argv1: {argv1}");
    assert!(read(dir.path(), "stdin.1").contains("u1"));

    // Call 2: first extension → persisted full send (no resume yet).
    let argv2 = read(dir.path(), "argv.2");
    assert!(!argv2.contains("--no-session-persistence"), "argv2: {argv2}");
    assert!(!argv2.contains("--resume"), "argv2: {argv2}");
    let stdin2 = read(dir.path(), "stdin.2");
    assert!(stdin2.contains("u1") && stdin2.contains("t1"), "stdin2: {stdin2}");

    // Call 3: resume with suffix only; assistant turns skipped.
    let argv3 = read(dir.path(), "argv.3");
    assert!(argv3.contains("--resume sess-2"), "argv3: {argv3}");
    let stdin3 = read(dir.path(), "stdin.3");
    assert!(stdin3.contains("u2"), "stdin3: {stdin3}");
    assert!(!stdin3.contains("u1"), "stdin3 resent prefix: {stdin3}");
    assert!(!stdin3.contains("t1"), "stdin3 resent prefix: {stdin3}");
    assert!(!stdin3.contains("ok2"), "stdin3 resent assistant: {stdin3}");
}

#[tokio::test]
#[serial]
async fn history_rewrite_resets_to_ephemeral() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 0);
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );
    drain(&client, vec![Message::system("sys"), Message::user("u1")])
        .await
        .unwrap();
    // Not an extension: same length, different content (curation rewrote history).
    drain(&client, vec![Message::system("sys"), Message::user("rewritten")])
        .await
        .unwrap();
    let argv2 = read(dir.path(), "argv.2");
    assert!(argv2.contains("--no-session-persistence"), "argv2: {argv2}");
    assert!(read(dir.path(), "stdin.2").contains("rewritten"));
}

#[tokio::test]
#[serial]
async fn stream_error_resets_session_state() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 2); // call 2 fails
    let client = ClaudeCliClient::with_options(
        fake.to_str().unwrap(),
        "sonnet",
        ClaudeCliOptions {
            session_reuse: true,
            ..Default::default()
        },
    );
    let base = vec![Message::system("sys"), Message::user("u1")];
    drain(&client, base.clone()).await.unwrap();
    let mut ext = base.clone();
    ext.push(Message::assistant("ok1", None));
    ext.push(Message::user("u2"));
    assert!(drain(&client, ext.clone()).await.is_err()); // call 2: persisted attempt fails
    // Retry (as the loop would): state was reset → ephemeral full send again.
    drain(&client, ext).await.unwrap();
    let argv3 = read(dir.path(), "argv.3");
    assert!(argv3.contains("--no-session-persistence"), "argv3: {argv3}");
    assert!(read(dir.path(), "stdin.3").contains("u1"), "full resend expected");
}

#[tokio::test]
#[serial]
async fn reuse_off_is_always_ephemeral_full_send() {
    let dir = tempfile::tempdir().unwrap();
    let fake = write_recording_fake(dir.path(), 0);
    let client = ClaudeCliClient::new(fake.to_str().unwrap(), "sonnet"); // Default: reuse off
    let base = vec![Message::system("sys"), Message::user("u1")];
    drain(&client, base.clone()).await.unwrap();
    let mut ext = base.clone();
    ext.push(Message::assistant("ok1", None));
    ext.push(Message::user("u2"));
    drain(&client, ext).await.unwrap();
    let argv2 = read(dir.path(), "argv.2");
    assert!(argv2.contains("--no-session-persistence"), "argv2: {argv2}");
    assert!(read(dir.path(), "stdin.2").contains("u1"), "full resend expected");
}
```

Note `write_fake` takes `&str` today — change its signature to `fn write_fake(script: &str)` call sites or pass `&script`; keep it compiling.

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p agent-model claude_cli
```

Expected: FAIL — `session_reuse_walks_ephemeral_persisted_resume` (no resume logic yet: call 2/3 argv still contain `--no-session-persistence`).

- [ ] **Step 3: Implement the state machine**

Add:

```rust
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
struct SessionState {
    /// Captured from the init event of a persisted spawn; needed to resume.
    session_id: Option<String>,
    /// Whether the CLI wrote this session to disk (resumable).
    persisted: bool,
    /// One hash per transcript message this session has covered.
    fingerprints: Vec<u64>,
}

enum SpawnPlan {
    /// Full transcript, `--no-session-persistence` (pre-optimization behavior).
    FreshEphemeral,
    /// Full transcript, persistence on; the init event's session_id is recorded.
    /// Costs one extra full send per session so one-shot callers (compaction,
    /// evals) never write session files to disk.
    FreshPersisted,
    /// `--resume <id>`; pipe only `messages[suffix_start..]`, assistant turns
    /// skipped (the CLI session already holds its own replies).
    Resume {
        session_id: String,
        suffix_start: usize,
    },
}

fn fingerprint(m: &Message) -> u64 {
    let mut h = DefaultHasher::new();
    std::mem::discriminant(&m.role).hash(&mut h);
    m.name.hash(&mut h);
    m.content.hash(&mut h);
    m.reasoning.hash(&mut h);
    h.finish()
}

/// `new` strictly extends `old`: longer, and byte-identical on the shared prefix.
fn is_strict_extension(old: &[u64], new: &[u64]) -> bool {
    new.len() > old.len() && new[..old.len()] == *old
}
```

`plan_spawn` on `ClaudeCliClient`:

```rust
/// Decide how to spawn for this transcript and the state to commit on success.
fn plan_spawn(&self, messages: &[Message]) -> (SpawnPlan, SessionState) {
    let fps: Vec<u64> = messages.iter().map(fingerprint).collect();
    let fresh = |persisted: bool| SessionState {
        session_id: None,
        persisted,
        fingerprints: fps.clone(),
    };
    if !self.opts.session_reuse {
        return (SpawnPlan::FreshEphemeral, fresh(false));
    }
    let st = self.state.lock().expect("session state lock").clone();
    match st {
        Some(s) if is_strict_extension(&s.fingerprints, &fps) => {
            if !s.persisted {
                // First extension: pay one full send to make the session resumable.
                return (SpawnPlan::FreshPersisted, fresh(true));
            }
            let suffix_start = s.fingerprints.len();
            let suffix_has_content = messages[suffix_start..]
                .iter()
                .any(|m| m.role != Role::Assistant);
            match (s.session_id.clone(), suffix_has_content) {
                (Some(id), true) => (
                    SpawnPlan::Resume {
                        session_id: id.clone(),
                        suffix_start,
                    },
                    SessionState {
                        session_id: Some(id),
                        persisted: true,
                        fingerprints: fps,
                    },
                ),
                // No id captured or assistant-only suffix: degrade safely.
                _ => (SpawnPlan::FreshEphemeral, fresh(false)),
            }
        }
        // First call, or history was rewritten (curation/compaction): start over.
        _ => (SpawnPlan::FreshEphemeral, fresh(false)),
    }
}
```

Rewire `stream()`:

```rust
async fn stream(
    &self,
    req: CompletionRequest,
) -> Result<BoxStream<'static, Result<Chunk, ModelError>>, ModelError> {
    let (plan, mut pending) = self.plan_spawn(&req.messages);

    let prompt = match &plan {
        SpawnPlan::Resume { suffix_start, .. } => {
            // The CLI session already holds its own assistant turns.
            let suffix: Vec<Message> = req.messages[*suffix_start..]
                .iter()
                .filter(|m| m.role != Role::Assistant)
                .cloned()
                .collect();
            render_transcript(&suffix)
        }
        _ => render_transcript(&req.messages),
    };

    let mut cmd = self.base_command();
    match &plan {
        SpawnPlan::FreshEphemeral => {
            cmd.arg("--no-session-persistence");
        }
        SpawnPlan::FreshPersisted => {}
        SpawnPlan::Resume { session_id, .. } => {
            cmd.arg("--resume").arg(session_id);
        }
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| ModelError::Process(format!("spawn {}: {e}", self.binary)))?;

    // Feed the prompt on a separate task so a large prompt can't deadlock
    // against the child filling its stdout pipe.
    let mut stdin = child.stdin.take().expect("stdin piped");
    tokio::spawn(async move {
        let _ = stdin.write_all(prompt.as_bytes()).await;
    });

    let stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");
    let stderr_task = tokio::spawn(async move {
        let mut buf = String::new();
        let _ = stderr.read_to_string(&mut buf).await;
        buf
    });

    let state = Arc::clone(&self.state);
    let track_state = self.opts.session_reuse;
    let stream = async_stream::stream! {
        let mut parser = EventParser::new();
        let mut lines = BufReader::new(stdout).lines();
        let mut failed = false;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => match parser.parse_line(&line) {
                    Ok(chunks) => {
                        for c in chunks {
                            yield Ok(c);
                        }
                    }
                    Err(e) => {
                        failed = true;
                        yield Err(e);
                        break;
                    }
                },
                Ok(None) => break, // stdout EOF
                Err(e) => {
                    failed = true;
                    yield Err(ModelError::Stream(e.to_string()));
                    break;
                }
            }
        }

        if !failed {
            match child.wait().await {
                Ok(status) if status.success() => {
                    if track_state {
                        // Prefer the id the CLI just reported (a resume may
                        // continue under the same id or, with future CLIs, a
                        // forked one — the init event is authoritative).
                        pending.session_id =
                            parser.session_id.take().or(pending.session_id.take());
                        *state.lock().expect("session state lock") = Some(pending);
                    }
                    return;
                }
                Ok(status) => {
                    failed = true;
                    let buf = stderr_task.await.unwrap_or_default();
                    yield Err(ModelError::Process(
                        format!("claude exited ({status}): {}", buf.trim())));
                }
                Err(e) => {
                    failed = true;
                    yield Err(ModelError::Process(e.to_string()));
                }
            }
        }
        if failed && track_state {
            // Reset so the loop's retry lands on a fresh full send — this IS
            // the "transparent fresh-session retry" (spec §2): Process/Stream
            // errors are Retryable, and the retry sees state == None.
            *state.lock().expect("session state lock") = None;
        }
    };
    Ok(stream.boxed())
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p agent-model claude_cli
```

Expected: PASS — all four new proc tests plus everything from Tasks 3–4.

- [ ] **Step 5: Run the whole crate + clippy**

```bash
cargo test -p agent-model && cargo clippy -p agent-model -- -D warnings
```

Expected: PASS, no warnings.

- [ ] **Step 6: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-model/src/claude_cli.rs
git commit -m "feat(model): claude-cli delta resume (ephemeral -> persisted -> resume state machine)"
```

---

### Task 6: Config knobs + plumbing

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/runtime_config.rs`
- Modify: `agent/crates/agent-runtime-config/src/lib.rs`
- Modify: `agent/crates/agent-cli/src/main.rs:186` (build_model call)
- Modify: `agent/crates/agent-server/src/runtime.rs:348` (build_model call)

**Interfaces:**
- Consumes: `ClaudeCliOptions` (Task 4, re-exported from `agent_model`).
- Produces: `RuntimeConfig` fields `claude_session_reuse: bool` (default **true**), `claude_effort: Option<String>`, `claude_fallback_model: Option<String>`; `pub fn claude_cli_opts(cfg: &RuntimeConfig) -> ClaudeCliOptions`; `build_model(backend, base_url, model, claude_binary, api_key, claude: ClaudeCliOptions)`.

- [ ] **Step 1: Write the failing config tests**

In `runtime_config.rs` tests:

```rust
#[test]
fn claude_knobs_default_reuse_on_and_no_flags() {
    let c = RuntimeConfig::default();
    assert!(c.claude_session_reuse);
    assert_eq!(c.claude_effort, None);
    assert_eq!(c.claude_fallback_model, None);
}

#[test]
fn validate_rejects_unknown_claude_effort() {
    let mut c = RuntimeConfig::default();
    c.claude_effort = Some("banana".into());
    let err = c.validate().unwrap_err();
    assert!(err.contains("claude_effort"), "got: {err}");
}

#[test]
fn validate_accepts_probed_effort_levels() {
    for level in EFFORT_LEVELS {
        let mut c = RuntimeConfig::default();
        c.claude_effort = Some((*level).into());
        assert!(c.validate().is_ok(), "level {level} should validate");
    }
}
```

- [ ] **Step 2: Run to verify failure**

```bash
cargo test -p agent-runtime-config claude_knobs
```

Expected: FAIL — fields not found.

- [ ] **Step 3: Implement fields, defaults, merge, validation**

In `RuntimeConfig` (after `preserve_thinking`):

```rust
/// claude-cli backend: resume the CLI session across calls when the
/// transcript extends append-only (delta resume). Ignored by openai.
#[serde(default = "default_true")]
pub claude_session_reuse: bool,
/// claude-cli backend: `--effort` level. None = CLI default.
#[serde(default)]
pub claude_effort: Option<String>,
/// claude-cli backend: `--fallback-model` when the primary is unavailable.
#[serde(default)]
pub claude_fallback_model: Option<String>,
```

In `Default` (`default_config()` literal, after `preserve_thinking: false,`):

```rust
claude_session_reuse: true,
claude_effort: None,
claude_fallback_model: None,
```

In `PartialRuntimeConfig` (after `preserve_thinking`):

```rust
claude_session_reuse: Option<bool>,
claude_effort: Option<String>,
claude_fallback_model: Option<String>,
```

In `merge()` (matching the existing arm style):

```rust
if let Some(v) = p.claude_session_reuse {
    self.claude_session_reuse = v;
}
if let Some(v) = p.claude_effort {
    self.claude_effort = Some(v);
}
if let Some(v) = p.claude_fallback_model {
    self.claude_fallback_model = Some(v);
}
```

Validation constant + check (**replace the list with the exact set probed in Task 1, Step 4** — recorded in `sources/probe-model-knobs-2-1-195.md`):

```rust
/// `--effort` values accepted by claude 2.1.195 (probed; see
/// docs/okf/claude-cli-headless/sources/probe-model-knobs-2-1-195.md).
pub const EFFORT_LEVELS: &[&str] = &["low", "medium", "high"];
```

In `validate()` (after the `repeat_penalty` check):

```rust
if let Some(e) = &self.claude_effort {
    if !EFFORT_LEVELS.contains(&e.as_str()) {
        return Err(format!(
            "claude_effort '{}' not recognized: use one of {}",
            e,
            EFFORT_LEVELS.join(" | ")
        ));
    }
}
```

Export `EFFORT_LEVELS` from the crate root alongside `RuntimeConfig` in `lib.rs`.

- [ ] **Step 4: Plumb `build_model`**

In `lib.rs`, import `ClaudeCliOptions` from `agent_model` and change:

```rust
/// Options for the claude-cli backend derived from config (openai ignores them).
pub fn claude_cli_opts(cfg: &RuntimeConfig) -> ClaudeCliOptions {
    ClaudeCliOptions {
        session_reuse: cfg.claude_session_reuse,
        effort: cfg.claude_effort.clone(),
        fallback_model: cfg.claude_fallback_model.clone(),
    }
}

/// Build the model client for the selected backend.
/// `claude-cli` ignores `base_url`/`api_key`; `openai` ignores `claude_binary`/`claude`.
pub fn build_model(
    backend: &str,
    base_url: &str,
    model: &str,
    claude_binary: &str,
    api_key: Option<String>,
    claude: ClaudeCliOptions,
) -> Arc<dyn ModelClient> {
    match backend {
        "claude-cli" => Arc::new(ClaudeCliClient::with_options(claude_binary, model, claude)),
        _ => Arc::new(OpenAiCompatClient::new(
            base_url.to_string(),
            model.to_string(),
            api_key,
        )),
    }
}
```

`build_routed_model` passes `claude_cli_opts(cfg)` as the new final argument. Routed clients (subagent/compaction) get the same knobs — safe because non-extension transcripts always plan `FreshEphemeral` (no disk writes for one-shot callers).

Update both call sites — `agent/crates/agent-cli/src/main.rs:186` and `agent/crates/agent-server/src/runtime.rs:348` — by appending `claude_cli_opts(&cfg)` (Read each site first; the config variable may be named differently, e.g. `config`). Add `claude_cli_opts` to each file's existing `agent_runtime_config` import list. Update the construction-contract test near `lib.rs:390` the same way.

- [ ] **Step 5: Build the workspace and fix any remaining literal sites**

```bash
cargo build --workspace 2>&1 | grep -E "^error" | head -20
```

Expected: clean. If any `RuntimeConfig { .. }` full-struct literal errors on the three missing fields (candidate: `src/eval/config.rs`), add the same three defaults (`claude_session_reuse: true, claude_effort: None, claude_fallback_model: None`). If any other `build_model(` caller surfaces, append `claude_cli_opts(&cfg)`.

- [ ] **Step 6: Run tests**

```bash
cargo test -p agent-runtime-config && cargo test -p agent-model
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cd /home/kalen/rust-agent-runtime
git add agent/crates/agent-runtime-config agent/crates/agent-cli/src/main.rs agent/crates/agent-server/src/runtime.rs
git commit -m "feat(config): claude-cli knobs (session reuse, effort, fallback model) + build_model plumbing"
```

---

### Task 7: Full gate + live smoke

**Files:** none new.

**Interfaces:**
- Consumes: everything above.
- Produces: green `scripts/ci.sh`; a live one-turn smoke against the real CLI.

- [ ] **Step 1: Run the CI gate**

```bash
cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh
```

Expected: fmt + clippy + cargo test (agent/) + web typecheck/vitest all pass. Fix anything it flags (rustfmt may rewrap new code).

- [ ] **Step 2: Live smoke test against the real CLI**

```bash
cd /home/kalen/rust-agent-runtime/agent
cargo run -p agent-cli -- --backend claude-cli --model sonnet --workspace . <<< "Say exactly: smoke-ok. Do not use any tools."
```

Expected: `smoke-ok` streams incrementally (partial messages working) and the process exits cleanly. Then check a session file appeared only if a multi-call turn ran (single-turn = ephemeral, no session file) — `ls ~/.claude/projects/ | head` as a sanity note, not an assertion.

- [ ] **Step 3: Commit any gate fixes**

```bash
git add -A && git commit -m "chore: ci gate fixes for claude-cli optimization" || echo "nothing to fix"
```

---

## Self-review notes (done at authoring time)

- Spec coverage: §1 research/bundle → Tasks 1–2; §2 session reuse → Task 5 (+ knob in Task 6); §3 streaming/thinking/knobs → Tasks 3–4; §4 config/tests → Task 6; gate → Task 7. Error-handling table: resume failure/state reset → Task 5 Step 3 + `stream_error_resets_session_state`; unknown stream_event ignored → Task 3 parser `_ => {}` arms; stderr drain unchanged.
- Deferred-persistence refinement over the spec ("persist on second use"): spec §2 says persistence is dropped "only in reuse mode"; the plan sharpens this so one-shot callers (compaction, evals) never write session files while reuse stays default-on. Recorded in the bundle (`practices/delta-resume.md`).
- Fixture literals in Task 3 are explicitly subordinated to Task 1 captures (same convention the old code documented).
- Type consistency: `ClaudeCliOptions` fields (`session_reuse`, `effort`, `fallback_model`) match across Tasks 4/5/6; `EventParser::parse_line` signature consistent between Tasks 3/5; `EFFORT_LEVELS` defined and exported in Task 6 where its tests live.
