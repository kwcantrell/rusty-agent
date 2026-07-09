# Sub-agent Structured-Response Handoff (3B-1b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a named sub-agent declare a flat-object `response_format`; its handoff then becomes a guaranteed schema-valid JSON payload (via a synthetic `respond` tool), or a marked free-text fallback.

**Architecture:** A named spec's `response_format` (a flat, no-regex JSON-schema subset) is validated at config assembly and carried on `ResolvedSubAgent`. At dispatch, a child that has one gets (a) a synthetic `respond` tool whose input schema *is* the format and which writes the validated payload to a shared `ResponseHandle`, and (b) a `ResponseCapture` middleware that ends the child cleanly (`EndRun(StopReason::Stop)`) the moment the handle fills. The handoff render reads the handle: `Some` → single-line JSON payload (child prose severed) + footer; `None` → marked fallback. No wire change, no recursion, no new dependency.

**Tech Stack:** Rust (the `agent/` Cargo workspace). Crates touched: `agent-core` (validator, `RespondTool`, `ResponseCapture`, dispatch wiring, prompt clause), `agent-runtime-config` (config validation narrowing, assembly resolution, config example, soak).

## Global Constraints

- **Two Cargo workspaces.** All work here is in the `agent/` workspace. `cargo` commands run from `/home/kalen/rust-agent-runtime/agent`. `-p <crate>` targets this workspace only.
- **Dep direction:** `agent-runtime-config` depends on `agent-core` (not vice-versa). The shared validator lives in `agent-core` and is called from both the `respond` tool (same crate) and config validation (`agent-runtime-config`). Never add an `agent-core → agent-runtime-config` dep (cycle).
- **Flat dialect only (owner gate 2026-07-09):** top-level closed object (`"type":"object"`, `"additionalProperties":false`); property types ∈ {scalar, enum, array-of-scalar}; `required`; property-count cap `MAX_RESPONSE_SCHEMA_PROPERTIES = 64`. Nested objects, array-of-object, `pattern`/regex/`format`, and combinators (`$ref`/`allOf`/`anyOf`/`oneOf`/`not`/`$defs`) are **config errors**. No recursion in the checker.
- **No new dependency, no regex engine.** The checker is an in-house flat pass.
- **Byte-identical when unset:** `general-purpose` and any named spec **without** `response_format` must produce an identical child stack and handoff to 3B-1 (commit 4cf682d). This is the headline regression.
- **`respond` is a reserved tool name**, registered directly into the child registry (exempt from the `tools` allowlist), never listed in a spec's `tools`.
- **Conventional commits** (`feat(core): …`, `test(config): …`). **Every task ships tests.** Locate quoted code **by content**, not line number — anchors drift.
- **CI gate:** `bash scripts/ci.sh` must pass before the branch is done.

---

## File Structure

- **Create** `agent/crates/agent-core/src/response_format.rs` — the whole feature's owned module: `ResponseHandle` type, `MAX_RESPONSE_SCHEMA_PROPERTIES`, `validate_schema` (config-time well-formedness), `validate_payload` (runtime), `RESPOND_TOOL_NAME`, and `RespondTool`. One responsibility: structured-response schema + tool.
- **Modify** `agent-core/src/lib.rs` — declare `mod response_format;` and re-export its public items + `ResponseCapture` + `RESPONSE_FORMAT_CLAUSE` at crate root (mirroring existing `SUBAGENT_PREAMBLE` / `ToolCallLimit` exports).
- **Modify** `agent-core/src/dispatch.rs` — add `RESPONSE_FORMAT_CLAUSE` const; add `response_format` field to `ResolvedSubAgent`; wire the handle/tool/middleware/render into `execute()`.
- **Modify** `agent-core/src/middleware.rs` — add the `ResponseCapture` middleware (next to `ToolCallLimit`).
- **Modify** `agent-runtime-config/src/assemble.rs` — resolve `spec.response_format` onto `ResolvedSubAgent` and append `RESPONSE_FORMAT_CLAUSE` to the composed child `system_prompt` when set.
- **Modify** `agent-runtime-config/src/runtime_config.rs` — narrow the reserved-field reject (accept + validate `response_format`; still reject `permissions`/`middleware`/`skills`); reject `respond` in a spec's `tools`.
- **Modify** `agent-runtime-config/config.example.toml` — commented `response_format` example.
- **Modify** `agent-runtime-config/tests/soak_live.rs` — add an `#[ignore]` live-rate + failure-cost soak.

---

## Task 0: Branch

- [ ] **Step 1: Create the feature branch off `main`**

```bash
cd /home/kalen/rust-agent-runtime
git checkout main
git checkout -b feature/subagent-structured-response
```

- [ ] **Step 2: Confirm clean baseline**

Run: `cd agent && cargo build -p agent-core -p agent-runtime-config`
Expected: builds clean at 4cf682d.

---

## Task A1: Flat-schema validator module

**Files:**
- Create: `agent/crates/agent-core/src/response_format.rs`
- Modify: `agent/crates/agent-core/src/lib.rs`

**Interfaces:**
- Produces: `pub type ResponseHandle = Arc<Mutex<Option<serde_json::Value>>>`; `pub const MAX_RESPONSE_SCHEMA_PROPERTIES: usize = 64`; `pub fn validate_schema(schema: &Value) -> Result<(), String>`; `pub fn validate_payload(schema: &Value, payload: &Value) -> Result<(), String>`. (`RespondTool` / `RESPOND_TOOL_NAME` are added to this same file in Task B1.)

- [ ] **Step 1: Write the failing tests**

Create `agent/crates/agent-core/src/response_format.rs` with only the test module for now:

```rust
//! Structured sub-agent responses (spec 3B-1b). A named sub-agent may declare a
//! FLAT-object `response_format`; the synthetic `respond` tool validates the
//! child's structured answer against it and writes it to a shared handle. No
//! nesting, no regex, no recursion — validation is a single flat pass.
use serde_json::Value;
use std::sync::{Arc, Mutex};

/// The single structured payload a `respond` call captures, shared between the
/// `RespondTool` (writer) and `ResponseCapture` / dispatch handoff (readers).
/// Mirrors `TodoHandle`. `None` until a valid `respond` call lands.
pub type ResponseHandle = Arc<Mutex<Option<Value>>>;

/// Max declared properties on a `response_format` object (flat dialect ceiling).
pub const MAX_RESPONSE_SCHEMA_PROPERTIES: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn good_schema() -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["severity", "files"],
            "properties": {
                "severity": {"type": "string", "enum": ["low", "high"]},
                "files": {"type": "array", "items": {"type": "string"}},
                "count": {"type": "integer"}
            }
        })
    }

    #[test]
    fn accepts_flat_object_schema() {
        assert!(validate_schema(&good_schema()).is_ok());
    }

    #[test]
    fn rejects_non_object_and_bad_top_level() {
        assert!(validate_schema(&json!([1, 2])).is_err());
        assert!(validate_schema(&json!({"type": "string"})).is_err());
        assert!(validate_schema(&json!({"type": "object", "properties": {}})).is_err()); // no additionalProperties:false
    }

    #[test]
    fn rejects_nesting_and_array_of_object() {
        let nested = json!({"type":"object","additionalProperties":false,
            "properties":{"inner":{"type":"object","additionalProperties":false,"properties":{}}}});
        assert!(validate_schema(&nested).is_err());
        let aoo = json!({"type":"object","additionalProperties":false,
            "properties":{"rows":{"type":"array","items":{"type":"object"}}}});
        assert!(validate_schema(&aoo).is_err());
    }

    #[test]
    fn rejects_regex_and_combinators() {
        let pat = json!({"type":"object","additionalProperties":false,
            "properties":{"s":{"type":"string","pattern":"^a+$"}}});
        assert!(validate_schema(&pat).is_err());
        let comb = json!({"type":"object","additionalProperties":false,
            "anyOf":[], "properties":{}});
        assert!(validate_schema(&comb).is_err());
    }

    #[test]
    fn rejects_required_naming_unknown_property_and_over_cap() {
        let bad_req = json!({"type":"object","additionalProperties":false,
            "required":["ghost"], "properties":{"real":{"type":"string"}}});
        assert!(validate_schema(&bad_req).is_err());
        let mut props = serde_json::Map::new();
        for i in 0..(MAX_RESPONSE_SCHEMA_PROPERTIES + 1) {
            props.insert(format!("p{i}"), json!({"type": "string"}));
        }
        let over = json!({"type":"object","additionalProperties":false,"properties":props});
        assert!(validate_schema(&over).is_err());
    }

    #[test]
    fn payload_valid_and_invalid() {
        let s = good_schema();
        assert!(validate_payload(&s, &json!({"severity":"low","files":["a.rs"]})).is_ok());
        assert!(validate_payload(&s, &json!({"files":["a.rs"]})).is_err()); // missing required
        assert!(validate_payload(&s, &json!({"severity":"low","files":["a.rs"],"x":1})).is_err()); // unknown key
        assert!(validate_payload(&s, &json!({"severity":"nope","files":[]})).is_err()); // enum miss
        assert!(validate_payload(&s, &json!({"severity":"low","files":[3]})).is_err()); // bad array element
        assert!(validate_payload(&s, &json!({"severity":"low","files":"a.rs"})).is_err()); // not an array
        assert!(validate_payload(&s, &json!("scalar")).is_err()); // not an object
    }
}
```

Add to `agent-core/src/lib.rs` (near the other `mod`/`pub use` lines). The crate re-exports submodules via `*` globs (e.g. `pub use dispatch::*;`, `pub use middleware::*;`), so a glob here auto-exports every `pub` item this module adds across A1/B1 (`validate_schema`, `validate_payload`, `ResponseHandle`, `MAX_RESPONSE_SCHEMA_PROPERTIES`, and later `RespondTool`, `RESPOND_TOOL_NAME`):

```rust
mod response_format;
pub use response_format::*;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-core response_format`
Expected: FAIL — `validate_schema` / `validate_payload` not found.

- [ ] **Step 3: Implement the checker**

Insert above the `#[cfg(test)]` module in `response_format.rs`:

```rust
const SCALAR_TYPES: [&str; 5] = ["string", "number", "integer", "boolean", "null"];
const BANNED_KEYS: [&str; 8] =
    ["pattern", "$ref", "allOf", "anyOf", "oneOf", "not", "$defs", "format"];

/// Config-time: is `schema` a well-formed FLAT-object response_format? (spec §2.5)
pub fn validate_schema(schema: &Value) -> Result<(), String> {
    let obj = schema.as_object().ok_or("must be a JSON object")?;
    if obj.get("type").and_then(Value::as_str) != Some("object") {
        return Err("top-level `type` must be \"object\"".into());
    }
    if obj.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err("must set `additionalProperties: false` (closed object)".into());
    }
    for k in BANNED_KEYS {
        if obj.contains_key(k) {
            return Err(format!("key `{k}` is not allowed"));
        }
    }
    let props = obj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("must have an object `properties`")?;
    if props.len() > MAX_RESPONSE_SCHEMA_PROPERTIES {
        return Err(format!(
            "too many properties ({} > {MAX_RESPONSE_SCHEMA_PROPERTIES})",
            props.len()
        ));
    }
    for (name, sub) in props {
        validate_property(name, sub)?;
    }
    if let Some(req) = obj.get("required") {
        let arr = req.as_array().ok_or("`required` must be an array")?;
        for r in arr {
            let rn = r.as_str().ok_or("`required` entries must be strings")?;
            if !props.contains_key(rn) {
                return Err(format!("`required` names unknown property `{rn}`"));
            }
        }
    }
    Ok(())
}

fn validate_property(name: &str, sub: &Value) -> Result<(), String> {
    let o = sub
        .as_object()
        .ok_or_else(|| format!("property `{name}` must be a schema object"))?;
    for k in BANNED_KEYS {
        if o.contains_key(k) {
            return Err(format!("property `{name}`: key `{k}` is not allowed"));
        }
    }
    match o.get("type").and_then(Value::as_str) {
        Some("object") => Err(format!("property `{name}`: nested object not allowed")),
        Some("array") => {
            let items = o
                .get("items")
                .and_then(Value::as_object)
                .ok_or_else(|| format!("property `{name}`: array needs object `items`"))?;
            let it = items.get("type").and_then(Value::as_str);
            if it == Some("object") || items.contains_key("properties") {
                return Err(format!("property `{name}`: array-of-object not allowed"));
            }
            if !matches!(it, Some(t) if SCALAR_TYPES.contains(&t)) {
                return Err(format!("property `{name}`: array `items.type` must be scalar"));
            }
            Ok(())
        }
        Some(t) if SCALAR_TYPES.contains(&t) => Ok(()),
        Some(t) => Err(format!("property `{name}`: unsupported type `{t}`")),
        None if o.contains_key("enum") => Ok(()), // enum-only (scalar literals)
        None => Err(format!("property `{name}`: needs a `type` or `enum`")),
    }
}

/// Runtime: does `payload` conform to the already-validated flat `schema`? (spec §2.5)
pub fn validate_payload(schema: &Value, payload: &Value) -> Result<(), String> {
    let sobj = schema.as_object().ok_or("schema not an object")?;
    let props = sobj
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("schema has no properties")?;
    let p = payload.as_object().ok_or("response must be a JSON object")?;
    if let Some(req) = sobj.get("required").and_then(Value::as_array) {
        for r in req {
            if let Some(rn) = r.as_str() {
                if !p.contains_key(rn) {
                    return Err(format!("missing required key `{rn}`"));
                }
            }
        }
    }
    for k in p.keys() {
        if !props.contains_key(k) {
            return Err(format!("unexpected property `{k}`"));
        }
    }
    for (name, val) in p {
        // `.get()` not `props[name]` — panic-free even if a caller ever passes an
        // unvalidated schema; the closed-object check above already guarantees the key.
        let sub = props
            .get(name)
            .and_then(Value::as_object)
            .ok_or("schema property not an object")?;
        check_value(name, sub, val)?;
    }
    Ok(())
}

fn check_value(
    name: &str,
    sub: &serde_json::Map<String, Value>,
    val: &Value,
) -> Result<(), String> {
    if let Some(e) = sub.get("enum").and_then(Value::as_array) {
        if !e.iter().any(|lit| lit == val) {
            return Err(format!("property `{name}`: value not in enum"));
        }
        return Ok(());
    }
    let scalar_ok = |t: Option<&str>, v: &Value| match t {
        Some("string") => v.is_string(),
        Some("boolean") => v.is_boolean(),
        Some("null") => v.is_null(),
        Some("integer") => v.is_i64() || v.is_u64(),
        Some("number") => v.is_number(),
        _ => false,
    };
    match sub.get("type").and_then(Value::as_str) {
        Some("array") => {
            let arr = val
                .as_array()
                .ok_or_else(|| format!("property `{name}`: expected array"))?;
            let it = sub
                .get("items")
                .and_then(Value::as_object)
                .and_then(|i| i.get("type"))
                .and_then(Value::as_str);
            for (i, el) in arr.iter().enumerate() {
                if !scalar_ok(it, el) {
                    return Err(format!("property `{name}`[{i}]: wrong element type"));
                }
            }
            Ok(())
        }
        t if scalar_ok(t, val) => Ok(()),
        _ => Err(format!("property `{name}`: wrong scalar type")),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-core response_format`
Expected: PASS (all 6 tests).

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/response_format.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): flat-object response_format schema validator (3B-1b A1)"
```

---

## Task A2: `ResolvedSubAgent.response_format` + assembly resolution + prompt clause

**Files:**
- Modify: `agent-core/src/dispatch.rs` (add `RESPONSE_FORMAT_CLAUSE` const; add field to `ResolvedSubAgent`)
- Modify: `agent-core/src/lib.rs` (re-export `RESPONSE_FORMAT_CLAUSE`)
- Modify: `agent-runtime-config/src/assemble.rs` (resolve field + append clause)
- Test: `agent-runtime-config/src/assemble.rs` tests

**Interfaces:**
- Consumes: `ResponseHandle` (Task A1, not needed here).
- Produces: `pub const RESPONSE_FORMAT_CLAUSE: &str`; `ResolvedSubAgent.response_format: Option<serde_json::Value>`. Task B2 reads `resolved.response_format`.

- [ ] **Step 1: Write the failing test**

In `agent-runtime-config/src/assemble.rs` tests, mirror the existing `named_subagent_resolves_into_dispatch_registry` test (which builds a config with `cfg()`, assembles via `assemble_loop(&c, parts(ws.path().into(), vec![]))`, and reads `built.subagent_registry.expect(...).get("triage")`). Add:

```rust
#[test]
fn response_format_resolves_and_appends_prompt_clause() {
    use crate::runtime_config::SubAgentSpec;
    let ws = tempfile::tempdir().unwrap();      // same tempdir idiom as the neighbor test
    let mut c = cfg();                          // the neighbor test's config builder
    c.named_subagents = vec![SubAgentSpec {
        name: "triage".into(),
        description: "Triage failures".into(),
        system_prompt: "You triage.".into(),
        tools: None,
        model: None,
        tool_call_limit: None,
        permissions: None,
        response_format: Some(serde_json::json!({
            "type": "object", "additionalProperties": false,
            "properties": {"summary": {"type": "string"}}
        })),
        middleware: None,
        skills: None,
    }];
    let built = assemble_loop(&c, parts(ws.path().into(), vec![]));
    let reg = built.subagent_registry.expect("registry built");
    let r = reg.get("triage").unwrap();
    assert_eq!(r.response_format.as_ref().unwrap()["type"], "object");
    assert!(r.system_prompt.contains(agent_core::RESPONSE_FORMAT_CLAUSE));
    assert!(r.system_prompt.contains(agent_core::SUBAGENT_PREAMBLE));
}
```

> `cfg()`, `parts(...)`, `assemble_loop`, and the `built.subagent_registry` field are exactly what `named_subagent_resolves_into_dispatch_registry` uses — copy that test's scaffold verbatim; do not invent helpers. (`subagent_registry` is `#[cfg(test)]`-populated on the assembled output.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-runtime-config response_format_resolves`
Expected: FAIL — `ResolvedSubAgent` has no field `response_format` / `RESPONSE_FORMAT_CLAUSE` not found.

- [ ] **Step 3: Add the const + struct field**

In `agent-core/src/dispatch.rs`, next to `pub const SUBAGENT_PREAMBLE`:

```rust
/// Appended to a named child's composed system prompt when its spec declares a
/// `response_format` (spec 3B-1b §2.2): the child returns its result by calling
/// the `respond` tool, not in prose.
pub const RESPONSE_FORMAT_CLAUSE: &str = "You MUST finish this task by calling the \
`respond` tool exactly once, passing your final answer as its arguments in the shape \
the tool's schema requires. Do not put your final answer in prose — only the \
`respond` call is returned to the parent. If a `respond` call is rejected as invalid, \
read the error and call `respond` again with corrected arguments.";
```

In the `ResolvedSubAgent` struct (same file), add the field after `tool_call_limit`:

```rust
    pub tool_call_limit: Option<usize>,
    /// The resolved flat `response_format` schema (spec 3B-1b §2.1); `None` ⇒ the
    /// child returns free prose as today.
    pub response_format: Option<serde_json::Value>,
```

No `lib.rs` change is needed for `RESPONSE_FORMAT_CLAUSE`: the crate re-exports dispatch items via `pub use dispatch::*;`, so the new `pub const` auto-exports at the crate root (same path `SUBAGENT_PREAMBLE` travels).

- [ ] **Step 4: Resolve the field + append the clause at assembly**

In `agent-runtime-config/src/assemble.rs`, in the loop that builds `agent_core::ResolvedSubAgent` from `cfg.named_subagents`, change the `system_prompt` composition and add the field. Locate the existing:

```rust
                    system_prompt: format!(
                        "{}\n\n{}",
                        spec.system_prompt,
                        agent_core::SUBAGENT_PREAMBLE
                    ),
                    tools: spec.tools.clone(),
```

Replace with:

```rust
                    system_prompt: {
                        let base = format!(
                            "{}\n\n{}",
                            spec.system_prompt,
                            agent_core::SUBAGENT_PREAMBLE
                        );
                        if spec.response_format.is_some() {
                            format!("{base}\n\n{}", agent_core::RESPONSE_FORMAT_CLAUSE)
                        } else {
                            base
                        }
                    },
                    tools: spec.tools.clone(),
```

And add, next to the other resolved fields (e.g. after `tool_call_limit: spec.tool_call_limit,`):

```rust
                    response_format: spec.response_format.clone(),
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cd agent && cargo test -p agent-runtime-config response_format_resolves`
Expected: PASS.

- [ ] **Step 6: Confirm no other `ResolvedSubAgent { … }` literal broke**

Run: `cd agent && cargo build -p agent-core -p agent-runtime-config`
Expected: builds. If a test literal of `ResolvedSubAgent` in `dispatch.rs` tests now misses `response_format`, add `response_format: None,` to it (Task B2 does this for the dispatch tests; fix any others here).

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-core/src/dispatch.rs agent/crates/agent-core/src/lib.rs agent/crates/agent-runtime-config/src/assemble.rs
git commit -m "feat(core): resolve response_format onto ResolvedSubAgent + prompt clause (3B-1b A2)"
```

---

## Task A3: Config validation — accept `response_format`, keep others reserved, reserve `respond`

**Files:**
- Modify: `agent-runtime-config/src/runtime_config.rs` (the `validate()` reserved-field block)
- Test: `agent-runtime-config/src/runtime_config.rs` tests

**Interfaces:**
- Consumes: `agent_core::validate_schema` (A1), `agent_core::RESPOND_TOOL_NAME` (added in B1 — see note in Step 3).

- [ ] **Step 1: Write the failing tests**

In `runtime_config.rs` tests, mirror the existing reserved-field test (which builds specs with `spec("name")` and configs with `cfg_with(vec![...])`, then calls `.validate()`). Add:

```rust
#[test]
fn accepts_valid_response_format_still_rejects_others() {
    let good = serde_json::json!({
        "type": "object", "additionalProperties": false,
        "required": ["summary"],
        "properties": {"summary": {"type": "string"}}
    });
    let mut s = spec("triage");
    s.response_format = Some(good.clone());
    assert!(cfg_with(vec![s]).validate().is_ok(), "valid flat response_format must be accepted");

    // ill-formed response_format (nested) → error naming the spec.
    let mut s2 = spec("triage");
    s2.response_format = Some(serde_json::json!({
        "type":"object","additionalProperties":false,
        "properties":{"inner":{"type":"object","additionalProperties":false,"properties":{}}}
    }));
    let e = cfg_with(vec![s2]).validate().unwrap_err();
    assert!(e.contains("response_format") && e.contains("triage"), "{e}");

    // permissions / middleware / skills still rejected.
    let mut sp = spec("triage"); sp.permissions = Some(serde_json::json!({}));
    assert!(cfg_with(vec![sp]).validate().is_err());
    let mut sm = spec("triage"); sm.middleware = Some(vec!["x".into()]);
    assert!(cfg_with(vec![sm]).validate().is_err());
    let mut sk = spec("triage"); sk.skills = Some(vec!["x".into()]);
    assert!(cfg_with(vec![sk]).validate().is_err());
}

#[test]
fn reserves_respond_tool_name() {
    let mut s = spec("triage");
    s.tools = Some(vec!["respond".into()]);
    let e = cfg_with(vec![s]).validate().unwrap_err();
    assert!(e.contains("respond") && e.contains("reserved"), "{e}");
}
```

> `spec("triage")` (returns a valid `SubAgentSpec`) and `cfg_with(vec![...])` (returns a `RuntimeConfig` with those `named_subagents`) are the real helpers the adjacent reserved-field test uses — reuse them verbatim; do not invent `base()`/`sample_spec()`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd agent && cargo test -p agent-runtime-config -- accepts_valid_response_format reserves_respond`
Expected: FAIL — `validate()` still rejects any non-null `response_format`; no reserved-name check.

- [ ] **Step 3: Narrow the reject block**

In `runtime_config.rs`, find the reserved-field reject:

```rust
            // Reserved fields are inert in 3B-1 (spec §2.7 item 6).
            if s.permissions.is_some()
                || s.response_format.is_some()
                || s.middleware.is_some()
                || s.skills.is_some()
            {
                return Err(format!(
                    "named_subagents['{}']: permissions/response_format/middleware/skills are not supported in 3B-1 (see 3B-1b/3B-1c)",
                    s.name
                ));
            }
```

Replace with:

```rust
            // permissions/middleware/skills remain inert (3B-1c / dropped, spec §0).
            if s.permissions.is_some() || s.middleware.is_some() || s.skills.is_some() {
                return Err(format!(
                    "named_subagents['{}']: permissions/middleware/skills are not supported (see 3B-1c)",
                    s.name
                ));
            }
            // 3B-1b: response_format is accepted, validated as a flat-object schema.
            if let Some(rf) = &s.response_format {
                agent_core::validate_schema(rf)
                    .map_err(|e| format!("named_subagents['{}']: response_format {e}", s.name))?;
            }
            // `respond` is the reserved synthetic structured-response tool name.
            if let Some(tools) = &s.tools {
                if tools.iter().any(|t| t == agent_core::RESPOND_TOOL_NAME) {
                    return Err(format!(
                        "named_subagents['{}']: `respond` is a reserved tool name",
                        s.name
                    ));
                }
            }
```

> `agent_core::RESPOND_TOOL_NAME` is defined in Task B1. If executing A3 before B1, temporarily inline the literal `"respond"` and the `reserves_respond_tool_name` test still passes; swap to the constant after B1. (Recommended order: A1 → A2 → B1 → A3 → B2 so the constant exists.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd agent && cargo test -p agent-runtime-config -- accepts_valid_response_format reserves_respond`
Expected: PASS.

- [ ] **Step 5: Update the pre-existing reserved-field test**

The existing test `validate_rejects_reserved_fields_and_model_endpoint_override` sets `s.response_format = Some(serde_json::json!({}))` and expects `Err`. After A3's narrowing, `{}` still errors — but now via `validate_schema` (no `"type":"object"`), so leaving that line would make the test **pass for the wrong reason** and stop testing the reserved-field path. **Remove the `response_format = Some(json!({}))` line entirely** (the other-three coverage now lives in `accepts_valid_response_format_still_rejects_others`), keeping the rest of that test (permissions/middleware/skills/model-endpoint) intact. Run: `cd agent && cargo test -p agent-runtime-config validate_rejects_reserved`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/crates/agent-runtime-config/src/runtime_config.rs
git commit -m "feat(config): accept+validate flat response_format, reserve respond name (3B-1b A3)"
```

---

## Task B1: `RespondTool` synthetic tool

**Files:**
- Modify: `agent-core/src/response_format.rs` (add `RESPOND_TOOL_NAME` + `RespondTool`)
- Modify: `agent-core/src/lib.rs` (re-export `RespondTool`, `RESPOND_TOOL_NAME`)
- Test: `agent-core/src/response_format.rs` tests

**Interfaces:**
- Consumes: `ResponseHandle`, `validate_payload` (A1).
- Produces: `pub const RESPOND_TOOL_NAME: &str = "respond"`; `pub struct RespondTool` with `pub fn new(schema: Value, handle: ResponseHandle) -> Self` implementing `agent_tools::Tool`. Task B2 registers it; Task A3 references the name constant.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `response_format.rs`:

```rust
    #[tokio::test]
    async fn respond_tool_writes_handle_on_valid_and_errs_on_invalid() {
        use agent_tools::{Tool, ToolCtx};
        use tokio_util::sync::CancellationToken;
        let schema = good_schema();
        let handle: ResponseHandle = Arc::new(Mutex::new(None));
        let tool = RespondTool::new(schema, handle.clone());
        assert_eq!(tool.name(), RESPOND_TOOL_NAME);

        let ctx = ToolCtx {
            workspace: std::env::temp_dir(),
            timeout: std::time::Duration::from_secs(5),
            cancel: CancellationToken::new(),
            sandbox: Arc::new(agent_tools::HostExecutor),
            backend: Arc::new(agent_tools::backend::HostBackend::new(std::env::temp_dir())),
            call_id: "r1".into(),
        };

        let ok = tool
            .execute(json!({"severity": "low", "files": ["a.rs"]}), &ctx)
            .await
            .unwrap();
        assert_eq!(ok.content, "response recorded");
        assert_eq!(handle.lock().unwrap().as_ref().unwrap()["severity"], "low");

        *handle.lock().unwrap() = None;
        let err = tool.execute(json!({"files": ["a.rs"]}), &ctx).await.unwrap_err();
        assert!(matches!(err, agent_tools::ToolError::InvalidArgs(_)));
        assert!(handle.lock().unwrap().is_none());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core respond_tool_writes_handle`
Expected: FAIL — `RespondTool` / `RESPOND_TOOL_NAME` not found.

- [ ] **Step 3: Implement `RespondTool`**

Add to `response_format.rs` (above the test module). Update the module's `use` line to include the tool types:

```rust
use agent_tools::{Access, Tool, ToolCtx, ToolError, ToolIntent, ToolOutput, ToolSchema};
use async_trait::async_trait;
```

```rust
/// The reserved name of the synthetic structured-response tool (spec §2.2).
pub const RESPOND_TOOL_NAME: &str = "respond";

/// The synthetic tool a named child with a `response_format` uses to return its
/// structured answer. Validates args against the flat schema and writes the
/// payload to the shared handle. A pure leaf: no dispatch power, no workspace
/// side-effects (spec §3 inv. 7).
pub struct RespondTool {
    schema: Value,
    handle: ResponseHandle,
    description: String,
}

impl RespondTool {
    pub fn new(schema: Value, handle: ResponseHandle) -> Self {
        Self {
            schema,
            handle,
            description: "Return your final answer as structured data matching this \
                tool's schema. Call this exactly once when the task is complete; its \
                arguments are returned to the parent as the result. If a call is \
                rejected as invalid, correct the arguments and call it again."
                .into(),
        }
    }
}

#[async_trait]
impl Tool for RespondTool {
    fn name(&self) -> &str {
        RESPOND_TOOL_NAME
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: RESPOND_TOOL_NAME.into(),
            description: self.description.clone(),
            parameters: self.schema.clone(),
        }
    }
    fn intent(&self, _args: &Value) -> Result<ToolIntent, ToolError> {
        Ok(ToolIntent {
            tool: RESPOND_TOOL_NAME.into(),
            access: Access::Read,
            paths: vec![],
            command: None,
            summary: "return the structured response".into(),
        })
    }
    async fn execute(&self, args: Value, _ctx: &ToolCtx) -> Result<ToolOutput, ToolError> {
        validate_payload(&self.schema, &args)
            .map_err(|e| ToolError::InvalidArgs(format!("respond: {e}")))?;
        *self.handle.lock().unwrap() = Some(args);
        Ok(ToolOutput {
            content: "response recorded".into(),
            display: None,
        })
    }
}
```

No `lib.rs` change is needed: A1 already added `pub use response_format::*;`, so the new `pub struct RespondTool` and `pub const RESPOND_TOOL_NAME` auto-export at the crate root (`agent_core::RespondTool`, `agent_core::RESPOND_TOOL_NAME`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd agent && cargo test -p agent-core respond_tool_writes_handle`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-core/src/response_format.rs agent/crates/agent-core/src/lib.rs
git commit -m "feat(core): RespondTool synthetic structured-response tool (3B-1b B1)"
```

---

## Task B2: `ResponseCapture` middleware + dispatch wiring + integration tests

**Files:**
- Modify: `agent-core/src/middleware.rs` (add `ResponseCapture`)
- Modify: `agent-core/src/lib.rs` (re-export `ResponseCapture`)
- Modify: `agent-core/src/dispatch.rs` (imports; mint handle; register `respond`; push `ResponseCapture`; handoff render)
- Test: `agent-core/src/dispatch.rs` tests

**Interfaces:**
- Consumes: `ResponseHandle`, `RespondTool`, `RESPOND_TOOL_NAME` (A1/B1); `resolved.response_format` (A2).
- Produces: `pub struct ResponseCapture` with `pub fn new(handle: ResponseHandle) -> Self`.

- [ ] **Step 1: Write the failing integration test (happy path)**

In `dispatch.rs` tests, add a helper + the first test:

```rust
    fn rf_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object", "additionalProperties": false,
            "required": ["summary"],
            "properties": {"summary": {"type": "string"}}
        })
    }

    fn resolved_with(rf: Option<serde_json::Value>, child: ScriptedModel, tcl: Option<usize>)
        -> std::collections::HashMap<String, ResolvedSubAgent> {
        let mut m = std::collections::HashMap::new();
        m.insert("triage".to_string(), ResolvedSubAgent {
            description: "Triage".into(),
            system_prompt: "You triage.".into(),
            tools: None,
            model: Arc::new(child),
            protocol: Arc::new(PassthroughProtocol),
            model_limit: None,
            max_tokens: None,
            tool_call_limit: tcl,
            response_format: rf,
        });
        m
    }

    #[tokio::test]
    async fn named_child_response_format_returns_severed_payload() {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "respond".into(), r#"{"summary":"done"}"#.into()),
        ]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(Some(rf_schema()), child, None)));
        let tool = DispatchAgentTool::new(deps);
        let out = tool
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await
            .unwrap();
        let line1 = out.content.lines().next().unwrap();
        let v: serde_json::Value = serde_json::from_str(line1).expect("line 1 is JSON");
        assert_eq!(v["summary"], "done");
        assert!(!out.content.contains("You triage"), "child prose must be severed");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd agent && cargo test -p agent-core named_child_response_format_returns_severed`
Expected: FAIL — `ResolvedSubAgent` literal missing `response_format` won't compile *or* (after A2) the `respond` tool isn't registered so the child can't call it → fallback, no JSON on line 1.

- [ ] **Step 3: Add the `ResponseCapture` middleware**

In `agent-core/src/middleware.rs`, next to `ToolCallLimit`:

```rust
/// Ends a named child cleanly the moment its `respond` payload is captured
/// (spec 3B-1b §2.3). Added to the child stack only when the spec declares a
/// `response_format`, and pushed AFTER `ToolCallLimit` so that — because
/// `fire_after_tools` iterates the stack in REVERSE and the first `EndRun` wins —
/// a captured response reports `Stop` even on a turn that also trips the call cap.
pub struct ResponseCapture {
    handle: crate::ResponseHandle,
}

impl ResponseCapture {
    pub fn new(handle: crate::ResponseHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Middleware for ResponseCapture {
    fn name(&self) -> &str {
        "response-capture"
    }
    async fn after_tools(&self, _cx: &mut RunCx<'_>) -> Flow {
        if self.handle.lock().unwrap().is_some() {
            Flow::EndRun(StopReason::Stop)
        } else {
            Flow::Continue
        }
    }
}
```

No `lib.rs` change is needed: the crate re-exports middleware via `pub use middleware::*;`, so the new `pub struct ResponseCapture` auto-exports at the crate root (same path `ToolCallLimit` travels — `dispatch.rs` reaches it as `crate::ResponseCapture`).

- [ ] **Step 4: Wire the handle + tool into `dispatch.rs::execute()`**

Add `ResponseCapture` and the response-format items to the `use crate::{…}` import block at the top of `dispatch.rs` (it already imports `ToolCallLimit`, `WriteTodosTool`, etc.):

```rust
    Middleware, OffloadConfig, RepairMiddleware, ResponseCapture, ResponseHandle, RespondTool,
    SessionArtifacts, StuckDetectionMiddleware, TodoHandle, ToolCallLimit, WriteTodosTool,
```

In `execute()`, right after the per-child `todos` handle block (where `write_todos` is conditionally re-registered), add:

```rust
        // Structured response (3B-1b §2.2): a named child with a response_format gets
        // a synthetic `respond` tool, registered DIRECTLY here (exempt from the `tools`
        // allowlist, like the context tools) so it is always callable. The handle is
        // read back at the handoff and by ResponseCapture.
        let response_handle: ResponseHandle = Arc::new(Mutex::new(None));
        let response_schema: Option<serde_json::Value> =
            resolved.and_then(|r| r.response_format.clone());
        if let Some(schema) = response_schema.clone() {
            // Resolved-registry collision guard (spec §2.2): no base/injected tool may
            // already own the reserved name. Benign by construction today (register is
            // last-wins, so `respond` would win anyway, and no host/context tool is
            // named `respond`), but assert it so a future collision fails loudly in
            // tests rather than silently shadowing a real tool.
            debug_assert!(
                reg.get(RESPOND_TOOL_NAME).is_none(),
                "reserved tool name `{RESPOND_TOOL_NAME}` collides with an existing child tool",
            );
            reg.register(Arc::new(RespondTool::new(schema, response_handle.clone())));
        }
```

In the child middleware block, after the `ToolCallLimit` push, add the `ResponseCapture` push (LAST, for reverse-order precedence):

```rust
        if let Some(cap) = resolved.and_then(|r| r.tool_call_limit) {
            child_mw.push(Arc::new(ToolCallLimit::with_cap(cap)));
        }
        // LAST in the vec ⇒ FIRST under fire_after_tools' reverse iteration ⇒ a
        // captured response wins a same-turn ToolCallLimit trip (reports Stop). §2.3.
        if response_schema.is_some() {
            child_mw.push(Arc::new(ResponseCapture::new(response_handle.clone())));
        }
```

- [ ] **Step 5: Change the handoff render (named exit, the `s.final_text` block near the end of `execute()`)**

Find the named handoff render:

```rust
        let content = if s.final_text.is_empty() {
            match budget_note {
                Some(note) => format!("{note}\n{footer}"),
                None => footer,
            }
        } else {
            match budget_note {
                Some(note) => format!("{note}\n{}\n\n{footer}", s.final_text),
                None => format!("{}\n\n{footer}", s.final_text),
            }
        };
```

Replace with:

```rust
        let content = if let Some(payload) = response_handle.lock().unwrap().take() {
            // §2.4 Some: single-line JSON payload (line 1), footer on later lines;
            // the child's pre-`respond` prose (final_text) is SEVERED. budget_note is
            // intentionally omitted: a captured payload means ResponseCapture ended the
            // run with Stop, so s.stop is never BudgetExhausted on this branch.
            let body = serde_json::to_string(&payload).unwrap_or_else(|_| "null".into());
            format!("{body}\n\n{footer}")
        } else if response_schema.is_some() {
            // §2.4 None + response_format set: marked, distinguishable free-text fallback.
            let marker = "[response_format: UNSATISFIED — free-text fallback]";
            match (s.final_text.is_empty(), budget_note) {
                (true, Some(note)) => format!("{note}\n{marker}\n{footer}"),
                (true, None) => format!("{marker}\n{footer}"),
                (false, Some(note)) => format!("{note}\n{}\n\n{marker}\n{footer}", s.final_text),
                (false, None) => format!("{}\n\n{marker}\n{footer}", s.final_text),
            }
        } else {
            // No response_format → byte-identical to 3B-1.
            if s.final_text.is_empty() {
                match budget_note {
                    Some(note) => format!("{note}\n{footer}"),
                    None => footer,
                }
            } else {
                match budget_note {
                    Some(note) => format!("{note}\n{}\n\n{footer}", s.final_text),
                    None => format!("{}\n\n{footer}", s.final_text),
                }
            }
        };
```

- [ ] **Step 6: Fix existing `ResolvedSubAgent { … }` test literals**

Any `ResolvedSubAgent { … }` literal in `dispatch.rs` tests (e.g. the named-resolution tests from 3B-1) now needs `response_format: None,`. Run: `cd agent && cargo build -p agent-core --tests` and add the field wherever the compiler flags a missing field.

- [ ] **Step 7: Run the happy-path test**

Run: `cd agent && cargo test -p agent-core named_child_response_format_returns_severed`
Expected: PASS.

- [ ] **Step 8: Add the remaining integration tests**

Append to `dispatch.rs` tests:

```rust
    #[tokio::test]
    async fn invalid_respond_retries_then_succeeds() {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "respond".into(), r#"{"wrong":1}"#.into()),
            Scripted::Call("c2".into(), "respond".into(), r#"{"summary":"ok"}"#.into()),
        ]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 4);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(Some(rf_schema()), child, None)));
        let out = DispatchAgentTool::new(deps)
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await.unwrap();
        let v: serde_json::Value = serde_json::from_str(out.content.lines().next().unwrap()).unwrap();
        assert_eq!(v["summary"], "ok");
    }

    #[tokio::test]
    async fn no_valid_respond_yields_marked_fallback() {
        let child = ScriptedModel::new(vec![Scripted::Text("prose answer, no tool".into())]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 2);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(Some(rf_schema()), child, None)));
        let out = DispatchAgentTool::new(deps)
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await.unwrap();
        assert!(out.content.contains("[response_format: UNSATISFIED"));
        assert!(serde_json::from_str::<serde_json::Value>(out.content.lines().next().unwrap()).is_err());
    }

    #[tokio::test]
    async fn respond_reachable_under_empty_tools_allowlist() {
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "respond".into(), r#"{"summary":"x"}"#.into()),
        ]);
        let mut m = resolved_with(Some(rf_schema()), child, None);
        m.get_mut("triage").unwrap().tools = Some(vec![]); // allowlist omits respond
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(m));
        let out = DispatchAgentTool::new(deps)
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await.unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(out.content.lines().next().unwrap()).unwrap()["summary"], "x");
    }

    #[tokio::test]
    async fn capture_wins_same_turn_tool_call_limit() {
        // tool_call_limit = 1: the respond call is the 1st (and cap-tripping) call.
        // ResponseCapture (pushed last) must win → footer reports Stop, not Error.
        let child = ScriptedModel::new(vec![
            Scripted::Call("c1".into(), "respond".into(), r#"{"summary":"done"}"#.into()),
        ]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(Some(rf_schema()), child, Some(1))));
        let out = DispatchAgentTool::new(deps)
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await.unwrap();
        assert!(out.content.contains("stop: Stop"), "capture must report Stop: {}", out.content);
        assert!(!out.content.contains("stop: Error"));
        // Precedence winning must also sever the payload on the same turn, not just
        // fix the stop-reason: line 1 is the JSON, not prose.
        let v: serde_json::Value = serde_json::from_str(out.content.lines().next().unwrap()).unwrap();
        assert_eq!(v["summary"], "done");
    }

    #[tokio::test]
    async fn named_child_without_response_format_is_byte_identical() {
        let child = ScriptedModel::new(vec![Scripted::Text("plain answer".into())]);
        let mut deps = exec_deps(ScriptedModel::new(vec![]), 3);
        deps.subagents = Arc::new(SubAgentRegistry::from_map(resolved_with(None, child, None)));
        let out = DispatchAgentTool::new(deps)
            .execute(serde_json::json!({"prompt":"go","subagent_type":"triage"}), &exec_ctx())
            .await.unwrap();
        assert_eq!(out.content, "plain answer\n\n[sub-agent: 1 turns, 0 tool calls, stop: Stop]");
    }
```

- [ ] **Step 9: Run all dispatch tests**

Run: `cd agent && cargo test -p agent-core dispatch`
Expected: PASS (new tests + all pre-existing dispatch tests, confirming the byte-identical no-`response_format` path).

- [ ] **Step 10: Commit**

```bash
git add agent/crates/agent-core/src/middleware.rs agent/crates/agent-core/src/lib.rs agent/crates/agent-core/src/dispatch.rs
git commit -m "feat(core): ResponseCapture + dispatch wiring for structured responses (3B-1b B2)"
```

---

## Task C: Config example, `#[ignore]` soak, full CI

**Files:**
- Modify: `agent-runtime-config/config.example.toml`
- Modify: `agent-runtime-config/tests/soak_live.rs`

- [ ] **Step 1: Document `response_format` in the config example**

In `config.example.toml`, under the existing `[[named_subagents]]` example, add a commented block:

```toml
# Optional: force this sub-agent to return a structured answer via the built-in
# `respond` tool. FLAT objects only — scalar / enum / array-of-scalar properties,
# `required`, and `additionalProperties = false`. No nesting, no regex/pattern.
# response_format = { type = "object", additionalProperties = false, \
#   required = ["severity", "summary"], properties = { \
#     severity = { type = "string", enum = ["low", "high"] }, \
#     files = { type = "array", items = { type = "string" } }, \
#     summary = { type = "string" } } }
```

> If TOML line-continuation is awkward in this file, write it as a single commented line — it is documentation, not parsed.

- [ ] **Step 2: Add the `#[ignore]` live soak**

This is **new harness work**, not copy-paste — it drives the real parent loop (via `agent.run_with_cancel`) to dispatch to a `response_format` sub-agent, and taps a sink to capture the resulting `dispatch_agent` tool-result. Mirror `soak_all_components_live` for the config + `assemble_loop`/`LoopParts` scaffold (the canonical example in this file), and mirror `SoakMonitor`'s `EventSink` impl for the exact `AgentEvent::ToolResult` variant fields.

The **config + registration** (fully concrete — the `from_launch` call is copied verbatim from `soak_all_components_live`; `max_depth = 1` so the parent may dispatch):

```rust
/// Live measurement (S1, spec §2.6): schema-valid rate + failure-tail turn cost of a
/// response_format sub-agent on the default model. Ignored — run explicitly with a
/// live server (AGENT_E2E_URL / AGENT_E2E_MODEL set).
#[tokio::test]
#[ignore = "live soak: requires AGENT_E2E_URL / AGENT_E2E_MODEL and a live server"]
async fn response_format_valid_rate_live() {
    let url = std::env::var("AGENT_E2E_URL").expect("set AGENT_E2E_URL");
    let model_name = std::env::var("AGENT_E2E_MODEL").expect("set AGENT_E2E_MODEL");
    let trials: usize = std::env::var("RF_TRIALS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);

    let schema = serde_json::json!({
        "type": "object", "additionalProperties": false,
        "required": ["severity", "summary"],
        "properties": {
            "severity": {"type": "string", "enum": ["low", "medium", "high"]},
            "summary": {"type": "string"}
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().to_path_buf();
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(), url.clone(), model_name.clone(), "native".into(), 8000,
    );
    cfg.memory = false;
    cfg.sandbox_mode = "off".into();
    cfg.max_turns = 6;
    cfg.max_depth = 1; // allow the parent to dispatch
    cfg.named_subagents = vec![agent_runtime_config::runtime_config::SubAgentSpec {
        name: "triager".into(),
        description: "Classify a software failure and summarize it.".into(),
        system_prompt: "You triage software failures. Given a failure description, \
            decide its severity and write a one-sentence summary.".into(),
        tools: Some(vec![]),           // no other tools — just reason + respond
        model: None,
        tool_call_limit: None,
        permissions: None,
        response_format: Some(schema.clone()),
        middleware: None,
        skills: None,
    }];
    cfg.validate().expect("config valid");
```

The **driver** (mirror the scaffold; the sink-tap is the one bit to write against `SoakMonitor`):

```rust
    // A capturing sink: records the content of every parent `dispatch_agent` tool
    // result. Implement EventSink by mirroring SoakMonitor's impl in this file and
    // matching on AgentEvent::ToolResult { name, .. } == "dispatch_agent" to stash
    // its content string. (Copy SoakMonitor's variant field names verbatim — they
    // are the source of truth for the AgentEvent shape.)
    // struct DispatchCapture { last: Mutex<Option<String>> }  impl EventSink for DispatchCapture { … }

    let mut valid = 0usize;
    let mut total_turns = 0usize;
    for i in 0..trials {
        let capture = Arc::new(DispatchCapture::default());
        // assemble_loop with LoopParts copied from soak_all_components_live, but
        // sink: capture.clone(); base_system_prompt instructs the parent to dispatch:
        //   "For each task, call dispatch_agent with subagent_type=\"triager\" and the
        //    failure text as prompt, then reply with its result."
        let built = assemble_loop(&cfg, /* LoopParts { … sink: capture.clone(), … } */ unimplemented_loopparts());
        let agent = built.loop_;
        let artifacts = Arc::new(SessionArtifacts::new());
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut ctx = CuratedContext::new(Message::system(built.system_prompt), artifacts, flag);
        let task = "Triage this failure: the CI build fails intermittently, ~1 in 5 runs, \
            on the integration test step, with no error in the logs.";
        let _ = agent
            .run_with_cancel(&mut ctx, task, tokio_util::sync::CancellationToken::new())
            .await;

        let out = capture.last.lock().unwrap().clone().unwrap_or_default();
        let line1 = out.lines().next().unwrap_or("");
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line1) {
            if agent_core::validate_payload(&schema, &v).is_ok() {
                valid += 1;
            }
        }
        if let Some(n) = out.split("[sub-agent: ").nth(1)
            .and_then(|s| s.split(" turns").next())
            .and_then(|s| s.trim().parse::<usize>().ok())
        {
            total_turns += n;
        }
        eprintln!("trial {i}: {}", if line1.starts_with('{') { "structured" } else { "fallback" });
    }
    eprintln!("response_format valid rate: {valid}/{trials}; avg sub-agent turns: {}",
        total_turns as f64 / trials.max(1) as f64);
    assert!(valid * 2 >= trials, "valid rate below 50% floor: {valid}/{trials}");
}
```

> The two placeholders — `DispatchCapture` (an `EventSink` mirroring `SoakMonitor`) and the `LoopParts { … }` block — are filled by copying `soak_all_components_live`'s `EventSink` impl and its `assemble_loop(&cfg, LoopParts { … })` call verbatim, swapping `sink: capture.clone()` and the dispatch-instructing `base_system_prompt`. This is the "real work" the coverage review flagged; it is `#[ignore]`d, so `ci.sh` never runs it — Step 3 only compiles it.

- [ ] **Step 3: Confirm it compiles (ignored, not run)**

Run: `cd agent && cargo test -p agent-runtime-config --test soak_live -- --list`
Expected: `response_format_valid_rate_live` listed as ignored; compiles clean.

- [ ] **Step 4: Run the full CI gate**

Run: `cd /home/kalen/rust-agent-runtime && bash scripts/ci.sh`
Expected: green — okf check + skills lint + fmt + clippy + `cargo test` (agent/) + src-tauri (conditional) + web typecheck/vitest.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-runtime-config/config.example.toml agent/crates/agent-runtime-config/tests/soak_live.rs
git commit -m "docs(config)+test(config): response_format example + ignored live soak (3B-1b C)"
```

---

## Self-Review (run after writing; performed inline)

**Spec coverage** — §2.5 dialect → A1; §2.1/§2.2 tool + handle → B1; §2.2 prompt clause + §2.4 render + reachability → A2/B2; §2.3 ResponseCapture + precedence → B2; §2.5 config narrowing + reserved name → A3; §2.6 S1 soak → C; §3 invariants (byte-identical, reserved-inert others) → A3/B2 tests; §5 test list → A1/A3/B1/B2/C. All covered.

**Placeholder scan** — no TBD/TODO; the two "match the neighboring helper" notes (A2 Step 1, A3 Step 1, C Step 2) point at concrete existing scaffolds the implementer copies, not invented APIs.

**Type consistency** — `ResponseHandle = Arc<Mutex<Option<Value>>>`, `validate_schema`/`validate_payload`, `RespondTool::new(schema, handle)`, `ResponseCapture::new(handle)`, `RESPOND_TOOL_NAME`, `RESPONSE_FORMAT_CLAUSE`, `ResolvedSubAgent.response_format` are named identically across A1→A2→A3→B1→B2→C.

**Ordering note** — recommended task order **A1 → A2 → B1 → A3 → B2 → C** so `agent_core::RESPOND_TOOL_NAME` (B1) exists before A3 references it (A3 Step 3 gives the inline-literal fallback if reordered).

**Reserved-name collision (spec §2.2, resolved-registry check).** Two layers: (1) config-time — a spec's `tools` may not list `respond` (Task A3). (2) dispatch-time — a `debug_assert!(reg.get(RESPOND_TOOL_NAME).is_none())` before registering `RespondTool` (Task B2 Step 4). This is benign by construction today (`register` is last-wins, so `respond` wins regardless, and no host/context/todos tool is named `respond`), so the invariant holds without a runtime error path; the `debug_assert` makes a *future* collision fail loudly in tests rather than silently shadowing a real tool. This satisfies §2.2's "checked against the resolved child registry" without adding a fallible assembly path.

**Plan-review dispositions (2026-07-09, both reviewers APPROVE-WITH-FIXES, all folded in):** lib.rs re-exports are `*` globs, not lists (phrasing fixed across A1/A2/B1/B2); Task C soak rewritten with the real `from_launch` config + `run_with_cancel` driver + sink tap (was notional); test-helper names corrected to the real scaffolds (`cfg()`/`assemble_loop`/`built.subagent_registry`; `spec()`/`cfg_with`); A3 Step 5 now *removes* the stale `response_format = Some(json!({}))` line; `validate_payload` uses `.get().ok_or()` not `[]` indexing (panic-safety); `capture_wins` test also asserts the payload survives; budget-note omission on the `Some(payload)` branch is commented as provably safe. All three pinned decisions (reverse-iteration precedence, render sever-site + Copy-safe fallback, panic-free validator + last-wins registration) were verified correct against live control flow by the architecture reviewer.
