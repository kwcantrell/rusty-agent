# Sub-agent structured-response handoff (deepagents refactor, Phase 3B-1b) — design

**Status:** BRAINSTORM-APPROVED 2026-07-09. Design approved in brainstorm dialogue;
awaiting adversarial spec panel (4 reviewers) + owner gate before `writing-plans`.
**Governing goal (owner, carried from Phase 2/3):** deepagents-style **modularity** —
a custom sub-agent changes runtime behavior via configuration, not code. This slice
adds one configured capability: a named sub-agent may declare the *shape* of its
answer.
**Knowledge base:** `docs/okf/deepagents-refactor/` (commit d997eec). Primary
practice source: `practices/subagent-context-quarantine.md`.
**Live-source baseline:** commit 4cf682d (3B-1 merged), re-read 2026-07-09 during
brainstorm. All `file:line` anchors are orientation only — **locate quoted code by
content before editing.**
**Builds on:** Phase-1 middleware seam (707d7fd), Phase-2 backend seam (71e23d1),
Phase-3A loop middleware wave (cb6ddf0), Phase-3B-1 named registry (4cf682d).
Preserves all prior-phase invariants (§3).
**Supersedes:** the `response_format` bullet of the 3B-1 spec §9. That bullet
correctly flagged the *original* prompt-and-parse mechanism as WRONG and demanded a
redesign; this spec is that redesign, and it discards prompt-and-parse entirely in
favor of a synthetic-tool mechanism (§2.2).

## 0. Scope

3B-1 reserved `response_format` as an inert field on `SubAgentSpec` (validation
hard-rejects any non-null value; 3B-1 spec §2.9). This slice **activates that one
field** and nothing else. `permissions` (→ 3B-1c), `middleware`, and `skills`
(dropped) stay validated-inert.

**IN (built this cycle):**
- Un-reserve `response_format` on `SubAgentSpec`; a **bounded, no-regex JSON-schema
  subset** (§2.5) as its accepted dialect, enforced at config validation.
- A synthetic **`respond`** tool + payload handle, added to a named child only when
  its spec sets `response_format` (§2.2).
- A **`ResponseCapture`** child-stack middleware for clean, deterministic
  termination (§2.3), conditionally added — symmetric with 3B-1's `ToolCallLimit?`.
- Handoff render: schema-valid JSON payload string, or a **marked** free-text
  fallback (§2.4).
- Scripted-model + validator + config-validation tests (§6); a stated success
  criterion (§2.6), measured in soak later, not in CI.

**OUT / deferred:**
- **No wire/event change.** The structured value rides the existing tool-result
  string (§2.4, decision (a)); promoting the payload onto a first-class typed
  channel is **3B-2** (typed subagent stream). §3 invariant "wire untouched" holds.
- **No regex/`pattern`/`format` support** in the schema dialect (§2.5) — banned, not
  bounded. A future slice may add it behind its own gate with explicit ReDoS bounds.
- **No native constrained decoding.** `CompletionRequest` gains no `response_format`
  field; neither backend is wired for grammar/json-schema-guided generation. See
  §2.1 "Approaches considered".
- **No dedicated retry counter.** Retries are the ordinary agentic loop, bounded by
  the child's existing `max_turns` / `tool_call_limit` (§2.2).
- **No live-model eval gating CI** (§2.6, S1).

## 1. Problem

3B-1 gave the runtime named sub-agents with their own `system_prompt` / `tools` /
`model` / `tool_call_limit`. Their **handoff is still free prose**: dispatch returns
`sink.summary().final_text` as a single tool-result string (`dispatch.rs` general
handoff ~L259, named handoff ~L734). A parent that dispatches a purpose-built child
("triage this failure", "extract the API surface") gets back whatever the child
happened to write, in whatever shape.

deepagents' `SubAgent` answers this with `response_format`: the child's result
conforms to a declared schema, so the caller — model *or* code — can rely on its
shape. The owner wants both consumers served (brainstorm: "Both"): the parent model
reading a reliably-shaped answer, and a code consumer (eval harness, a future 3B-2
stream consumer, an external caller over Tauri IPC) able to parse typed fields out of
the result.

## 2. Design

### 2.1 Approaches considered (why synthetic-tool)

Three mechanisms were weighed in brainstorm:

1. **Prompt-and-parse + bounded re-ask** (the 3B-1 §9 baseline). Inject the schema
   into the prompt, extract the last `{...}` span from `final_text`, validate,
   re-invoke on failure with a fresh sink. **Rejected:** `final_text` is a
   *cumulative token-stream tail* (`SubagentSink::summary` concatenates then trims
   segments — `dispatch.rs` ~L138-149), so parse is heuristic and fragile (prose
   braces, fenced code, nested JSON); the fresh-sink re-invoke is bespoke machinery
   built only to dodge the cumulative-counter double-count. High fragile surface.
2. **Synthetic final-response tool + payload handle.** `response_format` becomes the
   input schema of a `respond` tool; the child ends by calling it; args are
   structured JSON validated by the tool; a middleware ends the child cleanly.
   **Chosen** — see below.
3. **Native constrained decoding.** Plumb `response_format` → `CompletionRequest` →
   backends (llama.cpp GBNF/json-schema). **Rejected:** requires a new request field
   through both backends; **heterogeneous** — the Claude CLI backend has no free-form
   schema-constrained generation (only tool use), so its fork *collapses into
   approach 2 anyway*; and constrained generation fights tool-calling (you'd want to
   constrain only the final turn, which is not known a priori). Largest surface,
   backend-coupled, half-reinvents approach 2.

**Approach 2 wins on every axis that matters here:** no parse-from-prose (tool-call
args are already structured JSON); no bespoke re-ask path (retries are the ordinary
tool-error→model loop); **works identically on both backends** (tool-calling already
does); yields the typed payload "Both" needs for free; and it is how LangChain /
deepagents actually implement structured output under the hood (a bound tool).

### 2.2 Synthetic `respond` tool + payload handle

When a resolved named spec carries a `response_format`, the child's tool registry
gains a synthetic tool — working name **`respond`** (final name pinned at plan time;
must not collide with any registered tool — validated):

- **Input schema** = the spec's `response_format` (the bounded subset, §2.5),
  surfaced to the child model as the tool's `ToolSchema` parameters.
- **`execute`** validates the incoming args `Value` against the resolved schema using
  the in-house checker (§2.5):
  - **Valid** → writes the payload into an `Arc<Mutex<Option<serde_json::Value>>>`
    **response handle** (the established `TodoHandle = Arc<Mutex<…>>` pattern in
    dispatch) and returns a short ok `ToolOutput` (e.g. `"response recorded"`).
  - **Invalid** → returns `ToolError::InvalidArgs` whose message names the first
    validation failure (missing required key, wrong type, unexpected property under a
    closed object, depth). The message is **size-capped** by the child's
    `max_result_bytes` like any tool result. The model sees the error and **retries**
    — this *is* the retry path; there is **no dedicated retry counter**. Attempts are
    ordinary tool calls, bounded by the child's `max_turns` and (if set)
    `tool_call_limit`. §9(c)'s "charge re-ask against the child's budget" falls out
    for free.

The handle is minted in `execute()` of `DispatchAgentTool` (alongside the existing
per-child `todos` handle), cloned into the `respond` tool and into `ResponseCapture`
(§2.3), and read back after the child loop returns (§2.4). Only the **named** path
mints it; `general-purpose` and named specs without a `response_format` do not, and
their dispatch is byte-identical to 3B-1 (§3 inv. 1-2).

**System-prompt instruction (fix, mirrors MINOR-5 posture).** The resolved
`system_prompt` (composed at assembly, already includes `SUBAGENT_PREAMBLE`) gains a
**response-format clause** appended at assembly *only when `response_format` is set*:
a short, fixed instruction that the child must finish by calling `respond` with an
answer matching the tool's schema, and must not answer in prose. This composition
happens where the resolved system prompt is built (assembly, `assemble.rs` /
`ResolvedSubAgent`), not in the loop — keeping dispatch mechanics uniform.

### 2.3 `ResponseCapture` middleware — deterministic termination (T2)

Calling `respond` should *end* the child, deterministically and without wasting a
trailing turn. A tiny middleware does this, riding the exact seam 3A/3B-1 established:

- Added to the child stack **only when `response_format` is set**, making the stack
  `[curation, StuckDetection, Repair, ToolCallLimit?, ResponseCapture?]` — symmetric
  with the conditional `ToolCallLimit?` extension (3B-1 §2.5). When unset, the child
  stack is **byte-identical to 3B-1** (§3 inv. 2).
- `after_tools(&mut RunCx)` checks the response handle; once it is `Some`, returns
  **`Flow::EndRun(StopReason::Stop)`** — a *clean* completion, not an abort. This is
  the same `after_tools`→`EndRun` shape as `ToolCallLimit` (`middleware.rs` ~L595-603)
  and `StuckDetection`, but with `StopReason::Stop` (success) rather than
  `StopReason::Error`. Ordering after `Repair` and before/after `ToolCallLimit` is
  pinned at plan time (both are `after_tools` gates; a satisfied response should stop
  the run regardless of a same-turn tool-call-limit trip — plan resolves precedence).
- The middleware holds only the shared handle; it introduces no per-run typemap
  state (unlike `ToolCallLimit`'s counter). If the plan finds a typemap fits better,
  that is a plan-level choice; the observable contract is "handle populated ⇒ run
  ends `Stop` this turn."

### 2.4 Handoff render — payload string or marked fallback (scope decision (a))

After the child loop returns, the named handoff site (`dispatch.rs` ~L722-746) reads
the response handle:

- **`Some(payload)`** → the tool-result **string is the serialized JSON payload**
  (`serde_json::to_string`), followed by the existing sub-agent footer on its own
  line(s). The payload and footer are separable (footer is a bracketed final line),
  so a code consumer can strip the footer and `serde_json::from_str` the remainder,
  while the parent model reads the JSON directly. **No wire/event change** — this is
  the same single string the parent already receives (§3 inv. 6; typed channel is
  3B-2).
- **`None`** (the child stopped — turn/tool budget exhausted, or it simply ended
  without a valid `respond` call) → **marked free-text fallback**: the existing
  `final_text` render, plus a distinguishing marker line
  `[response_format: UNSATISFIED — free-text fallback]` in the footer region. This
  satisfies §9(d): fail-soft is **structurally distinguishable** (a code consumer
  sees a non-JSON body and/or the marker and knows the shape is not guaranteed),
  not a silent prose note.

`general-purpose` and named specs without `response_format` render exactly as 3B-1
(handle never minted → this branch not taken).

### 2.5 Schema dialect — bounded, no-regex subset

`response_format` is a `serde_json::Value` holding a **restricted JSON-schema
subset**, validated for well-formedness at **config-assembly time** (fail fast,
naming the offending spec) and used to validate child payloads at **runtime** (in
`respond.execute`). Both use one in-house recursive checker (no external crate, no
regex engine).

**Accepted keywords / types:**
- `type`: one of `object`, `array`, `string`, `number`, `integer`, `boolean`,
  `null` (and a small `[..]` union of these if cheap; plan decides).
- `object`: `properties` (map of name → subschema), `required` (array of names),
  `additionalProperties: false` honored (closed objects); default-closed vs
  default-open pinned at plan time (lean: reject unknown properties = closed, since
  the point is a *known* shape).
- `array`: `items` (a single subschema).
- `string`/`number`/`integer`/`boolean`/`null`: `enum` (array of literals).
- Nesting allowed to a **bounded depth** (`MAX_RESPONSE_SCHEMA_DEPTH`) and a bounded
  total **property/node count** (`MAX_RESPONSE_SCHEMA_NODES`), both defined
  constants, enforced at config validation.

**Rejected at config validation (→ config error naming the spec):**
- `pattern`, regex `format`, and any string-constraint that implies a regex engine —
  **banned outright.** This removes the ReDoS surface §9(c) flags *by construction*:
  there is no regex on any path, so there is nothing to bound.
- `$ref`, `allOf`/`anyOf`/`oneOf`/`not`, `$defs` and other combinators (unbounded /
  recursive expansion). A later slice may add a bounded subset behind its own gate.
- Schemas exceeding the depth or node ceilings.
- A `response_format` that is not a JSON object, or whose top-level `type` is absent.

**Runtime payload validation** (in `respond.execute`) checks a candidate `Value`
against the resolved subset: required keys present, types match, closed-object
unknown-key rejection, `enum` membership, `items` conformance, depth. First failure
→ a specific, size-capped `ToolError::InvalidArgs` message (§2.2).

This lifts `response_format` out of the 3B-1 RESERVED-inert set: the config
validation that today hard-rejects any non-null `response_format`
(`runtime_config.rs` ~L559-564) is **narrowed** to still reject `permissions`,
`middleware`, `skills`, but to *accept and dialect-validate* `response_format`.

### 2.6 Success criterion (S1)

**Target (stated, not CI-gated):** at least **90%** of dispatches to a named
sub-agent that declares a `response_format` yield a schema-valid payload within the
child's turn/tool budget, on the **default subagent model**, over a representative
set of small-object schemas. Rationale: the mechanism (a forced tool with a small
typed schema + natural retry) is exactly what current tool-using models do reliably;
the number is a floor to catch a mechanism regression, not a research target.

**How it is verified in 3B-1b:** the **mechanism** is proven deterministically with
scripted-model tests (§6): a scripted `respond` call produces the payload handoff; a
scripted invalid call produces a `ToolError` and a retry; budget exhaustion produces
the marked fallback. **Live-rate measurement is a soak follow-up**
(`soak_live.rs` / an eval task), gated and out of the fast `ci.sh` leg — consistent
with how the runtime already isolates live-model behavior from CI.

## 3. Do-not-regress / invariants

1. **`general-purpose` path byte-identical to 3B-1** — it has no `SubAgentSpec`, so
   no `response_format`, no handle, no `respond` tool, no `ResponseCapture`.
2. **Named specs without `response_format` are byte-identical to 3B-1** — child stack
   `[curation, StuckDetection, Repair, ToolCallLimit?]` unchanged; handoff render
   unchanged.
3. **Wire / event layer untouched** — the handoff remains the single tool-result
   string; `parent_id` forwarding, `sub{n}:` lineage, `SubagentSink`, all three
   frontends render as today. Typed payload channel is **3B-2** (§0).
4. **Guardrail/termination via `Flow::EndRun`** on the established seam;
   `ResponseCapture` uses `StopReason::Stop` (clean completion), never
   `BudgetExhausted` (3A invariant preserved).
5. **No calibration / estimator / pinned-block touch** — this is dispatch + config,
   not curation. The `respond` tool result and the payload string are ordinary
   tool-result content flowing through the existing offload/curation path.
6. **`permissions` / `middleware` / `skills` remain validated-inert** — only
   `response_format` is un-reserved. Any non-null value in the other three is still a
   config error (§2.5).
7. **Depth / transitive-scope / quarantine guarantees preserved** — `respond` is a
   leaf tool with no dispatch power; the response handle is per-child and never
   merged to the parent (transcript/todos quarantine, 3B-1 §3 inv. 8, unchanged).
8. **No new dependency, no regex engine** — the schema checker is in-house (§2.5).

## 4. Backward compatibility

- `response_format` unset (the default for every existing spec and for
  `general-purpose`) → behavior identical to 4cf682d.
- A config that used a non-null `response_format` would have been *rejected* by 3B-1
  (inert field), so no existing valid config changes meaning.
- `config.example.toml`'s `[[named_subagents]]` example may gain a commented
  `response_format` block (optional; plan decides).

## 5. Testing

- **Schema dialect (unit, `agent-runtime-config` or wherever the checker lands):**
  well-formedness — accept minimal object/array/enum schemas; reject `pattern`,
  combinators (`anyOf`/`$ref`), over-depth, over-node-count, non-object top-level.
- **Config validation (`agent-runtime-config`):** a non-null valid `response_format`
  now **accepted**; an ill-formed one → config error naming the spec; `permissions`/
  `middleware`/`skills` still rejected (guard against over-narrowing the reject).
- **Runtime payload validation (unit):** valid payload passes; missing required key,
  wrong type, unknown key under closed object, `enum` miss, over-depth → specific,
  size-capped errors.
- **Dispatch (`agent-core`, scripted models):**
  - named child with `response_format` + scripted `respond` call → handoff string is
    the JSON payload; footer separable; loop ends `Stop` via `ResponseCapture`.
  - scripted **invalid** `respond` args → `ToolError`, model retries, then a valid
    call succeeds.
  - child exhausts budget without a valid payload → **marked free-text fallback**
    (`[response_format: UNSATISFIED …]`).
  - `SUBAGENT_PREAMBLE` **and** the response-format clause both present in the
    resolved child system prompt (reuse the `SystemCapturingModel` pattern).
- **Regression:** named spec **without** `response_format` → child stack + handoff
  byte-identical to 3B-1 (headline); `general-purpose` unaffected.

## 6. Waves (for the plan — one spec, waved implementation)

Provisional; final decomposition at plan time.

- **Wave A — dialect + data model + validation.** The in-house bounded-subset schema
  checker (well-formedness + payload-validation entry points) with depth/node
  ceilings and `pattern`/combinator rejection; narrow 3B-1's reserved-field reject so
  `response_format` is accepted + dialect-validated (others still rejected);
  `ResolvedSubAgent` carries the resolved schema + a "has response_format" signal;
  the response-format system-prompt clause composed at assembly.
- **Wave B — dispatch integration.** Response handle minted per named child; synthetic
  `respond` tool (schema = resolved `response_format`, validates + writes handle);
  `ResponseCapture` middleware conditionally added to the child stack; handoff render
  (payload string vs marked fallback); scripted-model + validator + config tests.

## 7. For the plan reviewer

Standard single-reviewer plan review (spec coverage, decomposition, buildability).
A *lighter* adversarial architecture pass is warranted if real design decisions leak
into the plan — the notable ones here are (i) the schema-checker's exact accepted
subset and ceilings, (ii) `ResponseCapture` ordering vs `ToolCallLimit` in
`after_tools`, and (iii) the payload/footer separability contract. All three are
scoped in this spec; the plan pins them concretely.

## 8. Open items handed to the plan (not design forks)

- Final tool name (`respond` vs `final_response` vs `submit`) + collision check
  against registered tool names.
- Closed- vs open-object default in the dialect (lean closed).
- `ResponseCapture` vs `ToolCallLimit` precedence when both trip the same
  `after_tools`.
- Whether to add a commented `response_format` to `config.example.toml`.
- Exact `MAX_RESPONSE_SCHEMA_DEPTH` / `MAX_RESPONSE_SCHEMA_NODES` values.

## Panel & review log

### 2026-07-09 — Brainstorm (owner + agent)

Design settled via brainstorm dialogue. Decisions and their rationale:
- **Consumer = "Both"** (parent-model + code). → structured value must be
  machine-parseable, but (decision (a)) it rides the existing string handoff in this
  slice; the typed wire channel is deferred to 3B-2.
- **Mechanism = synthetic final-response tool** (approach 2), rejecting
  prompt-and-parse (fragile, bespoke re-ask) and native constrained decoding
  (backend-coupled, heterogeneous, half-reinvents approach 2).
- **Termination = `ResponseCapture` middleware** (T2), rejecting natural-stop (T1)
  for its overwrite/keep-going failure mode.
- **Fail-soft = marked free-text fallback**, rejecting hard `ToolError`.
- **Dialect = bounded no-regex subset**, rejecting a full-JSON-schema crate; `pattern`
  banned so ReDoS cannot occur.
- **Success = S1** (stated target + scripted-mechanism tests; live rate → soak),
  rejecting an in-slice live-model eval.
- **Retries = no dedicated counter** — the ordinary agentic loop, bounded by existing
  `max_turns`/`tool_call_limit`.

Next: adversarial spec panel (4 reviewers, distinct mandates), then owner gate.
