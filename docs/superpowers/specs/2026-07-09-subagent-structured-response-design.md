# Sub-agent structured-response handoff (deepagents refactor, Phase 3B-1b) — design

**Status:** PLAN-READY 2026-07-09. Brainstorm design approved; adversarial spec panel
(4 reviewers, distinct mandates — all **FIX-IN-PLACE, no BLOCKER**) run; owner gate
resolved the one escalation (schema dialect → **flat-object trim**, §2.5). All panel
dispositions folded in (see Panel & review log). Next: light-tier consistency read,
then `writing-plans`.
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
- Un-reserve `response_format` on `SubAgentSpec`; a **flat-object, no-regex
  JSON-schema dialect** (§2.5) as its accepted dialect, enforced at config
  validation.
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
- **No nested / array-of-object schemas** — flat single-level objects only (§2.5;
  owner gate 2026-07-09). Nesting is deferred behind a future gate exactly like
  `pattern`/combinators; the string handoff (decision (a)) makes nested shapes
  low-value in this slice anyway.
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
tool-error→model loop); **works on both backends** (tool-calling already does — with
one scoped divergence for *malformed-JSON* args on the prompted protocol, §2.6);
yields the typed payload "Both" needs for free; and it is how LangChain / deepagents
actually implement structured output under the hood (a bound tool).

### 2.2 Synthetic `respond` tool + payload handle

When a resolved named spec carries a `response_format`, the child's tool registry
gains a synthetic tool — working name **`respond`** (final name pinned at plan time).
It is **registered directly into the child registry like the context tools**
(`dispatch.rs` ~L578, `context_compact`), **exempt from the `spec.tools` allowlist** —
so a child that both sets a `tools` allowlist *and* a `response_format` can still call
the very tool it is required to call (a Requirements-mandate dead-mechanism trap
otherwise). `respond` (its final name) is a **reserved tool name**: a spec's `tools`
may not list it, and the name must not collide with any tool in the **resolved child
registry** — including the runtime-injected `context_compact` / `write_todos`, not
merely config-declared tools (config-time validation cannot see injected tools, so the
collision check is against the resolved set):

- **Input schema** = the spec's `response_format` (the bounded subset, §2.5),
  surfaced to the child model as the tool's `ToolSchema` parameters.
- **`execute`** validates the incoming args `Value` against the resolved schema using
  the in-house checker (§2.5):
  - **Valid** → writes the payload into an `Arc<Mutex<Option<serde_json::Value>>>`
    **response handle** (the established `TodoHandle = Arc<Mutex<…>>` pattern in
    dispatch) and returns a short ok `ToolOutput` (e.g. `"response recorded"`).
  - **Invalid** → returns `ToolError::InvalidArgs` whose message names the first
    validation failure (missing required key, wrong scalar type, unexpected property
    under a closed object, `enum` miss). The message is **size-capped** by the child's
    `max_result_bytes` like any tool result. The model sees the error and **retries**
    — this *is* the retry path; there is **no dedicated retry counter**. Attempts are
    ordinary tool calls, bounded by the child's `max_turns` (always set for children —
    default 10, a hard `for` loop, `loop_.rs` ~L723) and (if set) `tool_call_limit`.
    §9(c)'s "charge re-ask against the child's budget" falls out for free.
  - **Backend caveat (native vs prompted).** The valid/schema-invalid distinction
    above holds on **both** backends *only for JSON-well-formed args*. On the
    **prompted** protocol a **malformed-JSON** `respond` call is a *total* parse
    failure that routes to `RepairMiddleware` (one re-ask, then `GiveUp` →
    `Done(StopReason::Error)`), **never reaching `respond.execute`** and not counted
    against the tool budget. On the **native** protocol a malformed call is a
    per-call error and args still reach `execute`. The success criterion and tests
    (§2.6) are therefore **scoped to the native protocol** (the default local model's
    path); the prompted divergence is documented, not designed around.

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
  `StopReason::Error`.
- **Precedence (pinned, not deferred): `ResponseCapture` must run before
  `ToolCallLimit`** in `after_tools`, so that if the turn calling `respond` is *also*
  the limit-tripping turn, the run ends `Stop` (captured answer wins), never
  `EndRun(StopReason::Error)`. Note `fire_after_tools` runs the stack in **reverse**
  order and returns on the first `EndRun` (`loop_.rs` ~L359-373), so "runs first"
  translates to stack placement the plan pins accordingly. Without this, a satisfied
  response would render its payload but report `stop: Error`, misleading a stop-reason
  consumer.
- The middleware holds only the shared handle; it introduces no per-run typemap
  state (unlike `ToolCallLimit`'s counter). The observable contract is "handle
  populated ⇒ run ends `Stop` this turn."
- **Failure-arm safety (documented invariant):** the timeout / fatal-error dispatch
  arms return without consulting the handle, and that is safe *because*
  `ResponseCapture` returns `Ok` the same turn the handle is set — a
  handle-populated-then-timeout window does not exist. Stated so the plan does not
  "helpfully" add a handle read to those arms.

### 2.4 Handoff render — payload string or marked fallback (scope decision (a))

After the child loop returns, the named handoff site (`dispatch.rs` ~L722-746) reads
the response handle:

- **`Some(payload)`** → the tool-result **string is the serialized JSON payload**
  (`serde_json::to_string` — **single-line**, per the flat dialect always a
  `{...}` object), followed by the existing sub-agent footer on **later lines**. The
  **`final_text` body is NOT included** in this branch: the child's pre-`respond`
  reasoning prose (which `SubagentSink::summary` would otherwise return as the tail)
  must be **severed**, or it contaminates the "reliably-shaped answer." Separability
  contract: **the payload is the single first line (`to_string`, not
  `to_string_pretty`); the footer is the subsequent line(s).** A code consumer reads
  line 1 as the JSON body; the parent model reads the JSON directly. **No wire/event
  change** — the same single string the parent already receives (§3 inv. 3; a
  first-class typed channel is 3B-2, and robust typed access belongs there — this
  slice guarantees "line 1 is schema-valid JSON," not a general parser).
- **`None`** (the child stopped — turn/tool budget exhausted, or it simply ended
  without a valid `respond` call) → **marked free-text fallback**: the existing
  `final_text` render, plus a distinguishing marker line
  `[response_format: UNSATISFIED — free-text fallback]` in the footer region. This
  satisfies §9(d): fail-soft is **structurally distinguishable** (line 1 is not
  JSON, and the marker is present). **The marker is a human/model display hint, not a
  machine contract** — the authoritative "valid" signal is the *handle-derived JSON
  body*, never the marker's presence/absence (a child cannot manufacture a valid
  verdict by emitting the marker or fake JSON in prose; only `respond` writes the
  handle). A downstream typed status flag is 3B-2's to carry out-of-band.

`general-purpose` and named specs without `response_format` render exactly as 3B-1
(handle never minted → this branch not taken).

### 2.5 Schema dialect — flat object, no nesting, no regex (owner gate 2026-07-09)

`response_format` is a `serde_json::Value` holding a **flat, single-level JSON-schema
object**, validated for well-formedness at **config-assembly time** (fail fast,
naming the offending spec) and used to validate child payloads at **runtime** (in
`respond.execute`). Both use one small **non-recursive** in-house checker — no
external crate, no regex engine, **no recursion**.

**Accepted (the whole dialect):**
- Top level **must** be `{ "type": "object" }` with `"additionalProperties": false`
  (**closed object** — an unknown key in a payload is a validation failure; the point
  is a *known* shape).
- `properties`: a map of name → a **scalar / enum / array-of-scalar** subschema:
  - scalar `type` ∈ `string` / `number` / `integer` / `boolean` / `null`;
  - `enum` (array of scalar literals), with or without a `type`;
  - `array` whose `items` is a **scalar** subschema (`array-of-scalar` only).
- `required`: array of property names, each of which must appear in `properties`.
- A flat **property-count cap** (`MAX_RESPONSE_SCHEMA_PROPERTIES`, a defined
  constant), enforced at config validation.

Because the dialect is flat, **schema depth is structurally ≤ 2 and payload depth
≤ 2 — there is no recursion.** The depth×breadth product-blowup and the deep-payload
stack-overflow surfaces that a recursive checker would have to guard **do not exist**,
so the earlier draft's `MAX_RESPONSE_SCHEMA_DEPTH` / `MAX_RESPONSE_SCHEMA_NODES`
ceilings are **removed** (a single flat property-count cap replaces both). Stack
safety and complexity bounds are **by construction, not by bound** — this is the
owner-gate simplification (Panel log, Scope-mandate finding + Failure-mandate MAJORs
resolved together).

**Rejected at config validation (→ config error naming the spec):**
- **Nested objects** (a property whose `type` is `object`) and **array-of-object** (an
  `array` whose `items` is an object) — the flat / v2 boundary, deferred behind a
  future gate exactly like `pattern`/combinators below.
- `pattern`, regex `format`, and any string-constraint implying a regex engine —
  **banned outright.** Removes the ReDoS surface §9(c) flags *by construction*: no
  regex on any path, nothing to bound.
- `$ref`, `allOf`/`anyOf`/`oneOf`/`not`, `$defs` and other combinators.
- Schemas exceeding the property-count cap; a top-level that is not a closed object;
  a `required` name absent from `properties`.

**Runtime payload validation** (in `respond.execute`) is the **same flat pass** over
the incoming args `Value`: top level is an object; every `required` key present; each
present property matches its declared scalar/enum/array-of-scalar type; no unknown
keys (closed object); `enum` membership; `array` elements all match the scalar `items`
type. First failure → a specific, `max_result_bytes`-capped `ToolError::InvalidArgs`
(§2.2). Config-time well-formedness and runtime payload validation **share the one
flat-checker module** — not two divergent recursive walkers.

This lifts `response_format` out of the 3B-1 RESERVED-inert set: the config
validation that today hard-rejects any non-null `response_format`
(`runtime_config.rs` ~L558-564) is **narrowed** to still reject `permissions`,
`middleware`, `skills`, but to *accept and dialect-validate* `response_format`.

### 2.6 Success criterion (S1)

**Qualitative floor (stated, not CI-gated):** a named sub-agent that declares a
`response_format` should **reliably** yield a schema-valid payload within the child's
turn/tool budget, on the **default subagent model, native protocol** (§2.2 caveat),
over a representative set of small flat-object schemas. The mechanism (a forced tool
with a small typed schema + natural retry) is exactly what current tool-using models
do well. **No hard numeric target is fixed in this spec** — a number with no in-slice
measurement is a hostage to fortune; the *rate* is owned by the soak (below), which
sets/records it.

**How it is verified in 3B-1b:**
- The **mechanism** is proven deterministically with scripted-model tests (§5): a
  scripted `respond` call produces the payload handoff (line 1 is the JSON, no
  `final_text` contamination); a scripted schema-invalid call produces a `ToolError`
  and a retry; budget exhaustion produces the marked fallback.
- The **live rate** is measured by an **in-slice, `#[ignore]`d soak test**
  (`soak_live.rs` / an eval task) — runnable on demand at merge, **not** vaporware and
  **not** in the fast `ci.sh` leg, consistent with how the runtime isolates live-model
  behavior. The soak measures **both** the schema-valid rate **and the failure-tail
  cost** (see below).

**Cost-tail note (Failure-mandate residual):** a persistently-unsatisfiable child (a
weak model, or an awkward schema) is **distinguishable** (the `UNSATISFIED` marker)
but **costs the full `max_turns` budget every dispatch** — a cost-amplification tail,
not a resource-exhaustion bug (the loop is hard-bounded). The soak reports this tail
so the failure mode is measured, not just the success rate.

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
8. **No new dependency, no regex engine, no recursion** — the schema checker is an
   in-house flat pass (§2.5).

## 4. Backward compatibility

- `response_format` unset (the default for every existing spec and for
  `general-purpose`) → behavior identical to 4cf682d.
- A config that used a non-null `response_format` would have been *rejected* by 3B-1
  (inert field), so no existing valid config changes meaning.
- `config.example.toml`'s `[[named_subagents]]` example may gain a commented
  `response_format` block (optional; plan decides).

## 5. Testing

- **Schema dialect (unit, wherever the flat checker lands):** well-formedness —
  accept flat closed-object schemas with scalar/enum/array-of-scalar properties +
  `required`; reject **nested object**, **array-of-object**, `pattern`, combinators
  (`anyOf`/`$ref`), over-property-count, non-closed-object top-level, `required` name
  absent from `properties`.
- **Config validation (`agent-runtime-config`):** a non-null valid `response_format`
  now **accepted**; an ill-formed one → config error naming the spec; `permissions`/
  `middleware`/`skills` still rejected (guard against over-narrowing the reject).
- **Runtime payload validation (unit):** valid payload passes; missing required key,
  wrong scalar type, unknown key under closed object, `enum` miss, non-scalar array
  element → specific, `max_result_bytes`-capped errors.
- **Dispatch (`agent-core`, scripted models — native protocol):**
  - named child with `response_format` + scripted `respond` call → handoff **line 1 is
    the single-line JSON payload**, footer on later lines, and **no pre-`respond`
    token text** appears in the payload line (the sever test, Req MAJOR-2); loop ends
    `Stop` via `ResponseCapture`.
  - scripted **schema-invalid** (JSON-valid) `respond` args → `ToolError`, model
    retries, then a valid call succeeds.
  - `respond` reachable **even under a `spec.tools` allowlist that omits it** (the
    allowlist-exemption test); a spec listing the reserved `respond` name in `tools`
    → config/dispatch error.
  - `respond` capture **wins a same-turn `ToolCallLimit` trip** → handoff reports
    `stop: Stop`, not `Error` (the precedence test, §2.3).
  - child exhausts budget without a valid payload → **marked free-text fallback**
    (`[response_format: UNSATISFIED …]`), line 1 not JSON.
  - `SUBAGENT_PREAMBLE` **and** the response-format clause both present in the
    resolved child system prompt (reuse the `SystemCapturingModel` pattern).
- **Regression:** named spec **without** `response_format` → child stack + handoff
  byte-identical to 3B-1 (headline); `general-purpose` unaffected.
- **Soak (`#[ignore]`, live model):** schema-valid rate **and** failure-tail cost on
  the default subagent model (§2.6).

## 6. Waves (for the plan — one spec, waved implementation)

Provisional; final decomposition at plan time.

- **Wave A — dialect + data model + validation.** The in-house **flat, non-recursive**
  schema checker (shared well-formedness + payload-validation entry points) with the
  property-count cap and nested/array-of-object/`pattern`/combinator rejection; narrow
  3B-1's reserved-field reject so `response_format` is accepted + dialect-validated
  (others still rejected); `ResolvedSubAgent` carries the resolved schema + a "has
  response_format" signal; the response-format system-prompt clause composed at
  assembly.
- **Wave B — dispatch integration.** Response handle minted per named child; synthetic
  `respond` tool registered **outside the allowlist**, reserved name (schema = resolved
  `response_format`, validates + writes handle); `ResponseCapture` middleware
  conditionally added to the child stack, **ordered before `ToolCallLimit`**; handoff
  render (**severed** payload line vs marked fallback); scripted-model + validator +
  config tests; the `#[ignore]` soak.

## 7. For the plan reviewer

Standard single-reviewer plan review (spec coverage, decomposition, buildability).
A *lighter* adversarial architecture pass is warranted if real design decisions leak
into the plan — the notable ones here are (i) the schema-checker's exact accepted
subset and ceilings, (ii) `ResponseCapture` ordering vs `ToolCallLimit` in
`after_tools`, and (iii) the payload/footer separability contract. All three are
scoped in this spec; the plan pins them concretely.

## 8. Open items handed to the plan (not design forks)

- Final tool name (`respond` vs `final_response` vs `submit`) + reserved-name /
  collision check against the **resolved** child registry (incl. `context_compact` /
  `write_todos`), per §2.2.
- Whether to add a commented `response_format` to `config.example.toml`.
- Exact `MAX_RESPONSE_SCHEMA_PROPERTIES` value.

(Decided by the panel/gate, no longer open: closed-object is mandatory — not a
default; `ResponseCapture` runs **before** `ToolCallLimit` (§2.3); depth/node ceilings
removed with nesting (§2.5).)

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

### 2026-07-09 — Adversarial spec panel (4 reviewers) + owner gate

Four skeptical reviewers (Requirements / Assumptions / Failure & abuse / Scope &
simpler-design), opus-tier, each reading the spec + live source at 4cf682d. **All
four verdicts: FIX-IN-PLACE — no BLOCKER.** The synthetic-tool redesign was confirmed
sound at source and confirmed to genuinely fix the three §9 defects it supersedes.

**Escalated to the owner gate — resolved (schema dialect scope):**
- Scope-mandate recommended **trimming the dialect to flat-object-only**; this makes
  the Failure-mandate's two MAJORs (node-count product blowup; payload-recursion stack
  safety) **evaporate by construction** (no recursion) and removes the recursive
  in-house-checker tar-pit risk. It revises the brainstorm's "bounded subset with
  nesting," so it went to the gate. **Owner decision (2026-07-09): TRIM to
  flat-object** (§2.5) — top-level closed object, scalar/enum/array-of-scalar
  properties, `required`, one property-count cap, no recursion. Nested / array-of-object
  deferred behind a future gate (like `pattern`/combinators). One decision resolved a
  Scope MAJOR + two Failure MAJORs + the checker MINOR.

**Blockers/majors — fixed in place (this spec):**
- **Req MAJOR-2** — handoff must **sever** the `final_text` body when the handle is
  `Some`, else pre-`respond` prose contaminates the payload → §2.4 (normative) + sever
  test (§5).
- **Req MAJOR-1 / Scope F5** — the footer-separability contract was unreliable for the
  old dialect's non-object cases → §2.4 pins **single-line `to_string` payload = line
  1**; flat-object trim makes top-level always `{...}`; code-consumer claim scoped
  honestly (robust typed access is 3B-2).
- **Assumptions MAJOR** — "identical on both backends" overclaim: **malformed-JSON**
  `respond` args on the *prompted* protocol hit total-parse-failure → Repair GiveUp →
  `Error`, never reaching `execute`. Fixed: §2.1 claim softened, §2.2 backend caveat
  added, §2.6 success criterion + tests **scoped to native protocol**.
- **Failure MAJOR-1/MAJOR-2** — node-count global-accumulator + payload-recursion
  stack safety: **resolved by the flat-object trim** (no recursion, depth ≤ 2), not by
  adding bounds.

**Minors — fixed in place:**
- `respond` registered **outside the `tools` allowlist** + **reserved name**, collision
  checked against the **resolved** child registry incl. runtime-injected tools (Req
  MINOR-2/3, Failure MINOR-2) → §2.2, §8.
- `ResponseCapture` **before** `ToolCallLimit` in `after_tools` — pinned, not deferred
  (Req MINOR-4, Assumptions MINOR) → §2.3.
- Failure-arm handle-read safety documented (Assumptions MINOR) → §2.3.
- Fallback marker is a **display hint, not a machine contract** (Failure MINOR-3) →
  §2.4.
- Success criterion: **dropped the hard 90% number**, kept the qualitative floor, and
  the soak lands **in-slice as `#[ignore]`** measuring valid-rate **and** failure-tail
  cost (Scope F4, Req MINOR-1, Failure MINOR-1) → §2.6, §5.

**Minors — accepted as residual:**
- Never-satisfiable-child **cost-amplification tail** (full budget per dispatch,
  distinguishable via the marker) — bound is sound (`max_turns`); recorded in §2.6 and
  measured by the soak.

**Honest positives confirmed by the panel (kept as-is):** `after_tools→EndRun(Stop)`
cleanly ends the child with the `respond` result recorded; the handle is the sole
source of a "valid" verdict (no prose spoof); `respond` is a verified pure leaf (no
re-open of the 3B-1 escalation class); retries hard-bounded by `max_turns`;
`ResponseCapture` is *justified* (T1 natural-stop genuinely broken — a tool-call turn
always loops, no `Tool`→`Flow` channel); `general-purpose`/unset paths byte-identical.
