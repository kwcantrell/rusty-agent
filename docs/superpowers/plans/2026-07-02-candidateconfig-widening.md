# CandidateConfig Widening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add inherit-on-`None` `system_prompt` and `protocol` override fields to the eval `CandidateConfig`, wired into the live harness, so prompt/protocol become optimizer genome axes (enabler for items 3 and 5).

**Architecture:** Two additive serde-default `Option` fields + two resolver methods on `CandidateConfig` (unit-testable without the live harness); the `#[ignore]` harness in `tests/eval_context.rs` consumes the resolvers. Gate/admissibility untouched.

**Tech Stack:** Rust (`agent/` workspace), serde.

**Spec:** `docs/superpowers/specs/2026-07-02-candidateconfig-widening-design.md`

## Global Constraints

- `agent/` Cargo workspace (`cd agent`; `source ~/.cargo/env` if needed). `-p agent-runtime-config`.
- Additive serde-default fields only — existing CandidateConfig JSON must parse (frozen champion configs).
- Gate (`eval/gate.rs`) and admissibility (`eval/admissibility.rs`) MUST NOT change.
- Tool-description variants are OUT of scope (no seam; deferred).
- Conventional commits.

---

### Task 1: widen CandidateConfig + wire the harness

**Files:**
- Modify: `agent/crates/agent-runtime-config/src/eval/config.rs` (struct + `favorable` + new methods + tests)
- Modify: `agent/crates/agent-runtime-config/tests/eval_context.rs` (lift the prompt const, consume resolvers at ~142 protocol and ~210 base_system_prompt)

**Interfaces:**
- Produces: `CandidateConfig.system_prompt: Option<String>`, `CandidateConfig.protocol: Option<String>`, `resolved_system_prompt(&self, default) -> &str`, `resolved_protocol(&self, default) -> &str`.

- [ ] **Step 1: Write the failing unit tests**

In `eval/config.rs` (add a `#[cfg(test)] mod tests` if none exists, else extend):

```rust
#[cfg(test)]
mod widening_tests {
    use super::*;

    #[test]
    fn new_fields_default_to_none_and_inherit() {
        let cc = CandidateConfig::favorable(8192);
        assert!(cc.system_prompt.is_none());
        assert!(cc.protocol.is_none());
        assert_eq!(cc.resolved_system_prompt("BASE"), "BASE");
        assert_eq!(cc.resolved_protocol("native"), "native");
    }

    #[test]
    fn overrides_win_over_default() {
        let mut cc = CandidateConfig::favorable(8192);
        cc.system_prompt = Some("CANDIDATE PROMPT".into());
        cc.protocol = Some("prompted".into());
        assert_eq!(cc.resolved_system_prompt("BASE"), "CANDIDATE PROMPT");
        assert_eq!(cc.resolved_protocol("native"), "prompted");
    }

    #[test]
    fn json_missing_new_fields_parses_to_none() {
        // A pre-widening config (existing field set only) must still deserialize.
        let json = serde_json::to_value(CandidateConfig::favorable(8192)).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        obj.remove("system_prompt");
        obj.remove("protocol");
        let cc: CandidateConfig =
            serde_json::from_value(serde_json::Value::Object(obj)).unwrap();
        assert!(cc.system_prompt.is_none() && cc.protocol.is_none());
    }

    #[test]
    fn explicit_values_round_trip() {
        let mut cc = CandidateConfig::favorable(8192);
        cc.system_prompt = Some("P".into());
        cc.protocol = Some("prompted".into());
        let back: CandidateConfig =
            serde_json::from_str(&serde_json::to_string(&cc).unwrap()).unwrap();
        assert_eq!(back.system_prompt.as_deref(), Some("P"));
        assert_eq!(back.protocol.as_deref(), Some("prompted"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd agent && cargo test -p agent-runtime-config widening_tests`
Expected: compile error (no such fields/methods).

- [ ] **Step 3: Implement the struct fields + methods**

In `eval/config.rs`, add to the struct (after the memory knobs, before the closing brace):

```rust
    /// Override the base system prompt for this candidate. None = the harness
    /// default (inherit). A prompt-wording genome axis for the optimizer.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Override the tool-call protocol ("native" | "prompted"). None = inherit
    /// the harness default. A protocol-encoding genome axis.
    #[serde(default)]
    pub protocol: Option<String>,
```

In `favorable(window)`, add both to the constructed literal:

```rust
            system_prompt: None,
            protocol: None,
```

Add the resolver methods in the `impl CandidateConfig` block:

```rust
    /// The system prompt this candidate runs under: its override, else `default`.
    pub fn resolved_system_prompt<'a>(&'a self, default: &'a str) -> &'a str {
        self.system_prompt.as_deref().unwrap_or(default)
    }
    /// The protocol name this candidate runs under: its override, else `default`.
    pub fn resolved_protocol<'a>(&'a self, default: &'a str) -> &'a str {
        self.protocol.as_deref().unwrap_or(default)
    }
```

If any OTHER constructor of `CandidateConfig` exists (grep the crate for
`CandidateConfig {`), add the two fields there too.

- [ ] **Step 4: Run unit tests**

Run: `cd agent && cargo test -p agent-runtime-config`
Expected: PASS (new + all existing, incl. any frozen-config deserialization test).

- [ ] **Step 5: Wire the live harness**

In `tests/eval_context.rs`:

Lift the hardcoded prompt to a module const (top of the test module):

```rust
const EVAL_DEFAULT_PROMPT: &str =
    "You are a coding agent operating in a sandboxed workspace. Use the provided \
    tools to complete each task, then give a short final reply.";
```

At the protocol site (~line 142, the `from_launch` call): resolve and use it —

```rust
        let protocol = cc.resolved_protocol("native").to_string();
        let mut cfg = RuntimeConfig::from_launch(
            "openai".into(),
            url.clone(),
            model.clone(),
            protocol,
            cc.context_limit,
        );
```

At the `base_system_prompt` site (~line 210-213): replace the inline literal with

```rust
                base_system_prompt: cc.resolved_system_prompt(EVAL_DEFAULT_PROMPT).to_string(),
```

- [ ] **Step 6: Verify the harness still compiles**

Run: `cd agent && cargo test -p agent-runtime-config --no-run` (compiles the
`#[ignore]` live test without running it) and `cargo test -p agent-runtime-config`
(runs everything not ignored).
Expected: compiles clean; all non-ignored tests pass.

- [ ] **Step 7: Commit**

```bash
git add agent/crates/agent-runtime-config/src/eval/config.rs agent/crates/agent-runtime-config/tests/eval_context.rs
git commit -m "feat(eval): widen CandidateConfig with inherit-on-None system_prompt + protocol axes"
```

---

### Task 2: CI gate

- [ ] Run: `bash scripts/ci.sh`. Expected: green. Fix anything red.
