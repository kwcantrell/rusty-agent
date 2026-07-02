//! Data-driven adversarial command-policy corpus.
//!
//! Each row of `policy_corpus.tsv` is `expected<TAB>command`, where
//! `expected ∈ allow | ask | deny`. Every command is fed through the SAME engine
//! production wires (`agent-runtime-config`'s `assemble_loop`): a `RulePolicy`
//! built from `default_allowlist()` + `effective_denylist()`, checked with the
//! exact `ToolIntent` that `execute_command` produces (`ExecuteCommand::intent`).
//! So the corpus guards the wiring — default lists, intent shape, and the
//! `RulePolicy::check` ladder — not a re-implementation of the classifier.
//!
//! Adding a regression case is a one-line TSV addition. Expected values pin
//! CURRENT behavior; the file's comments document accepted asymmetries.

use agent_policy::{Decision, PolicyEngine, RulePolicy};
use agent_runtime_config::{default_allowlist, default_denylist, RuntimeConfig};
use agent_tools::shell::ExecuteCommand;
use agent_tools::Tool;
use std::path::PathBuf;

/// Build the command-policy engine EXACTLY as production does: the CLI seeds
/// `command_allowlist = default_allowlist()` and `command_denylist =
/// default_denylist()` (agent-cli/src/main.rs), then `assemble_loop`
/// (agent-runtime-config/src/assemble.rs) hands `RulePolicy` the allowlist
/// verbatim and `effective_denylist()` (= HARD_FLOOR ∪ user denylist) as the
/// denylist. Reproducing that here means the corpus fails if any of those wires
/// change, not just if the classifier does.
fn engine() -> RulePolicy {
    let mut cfg = RuntimeConfig::from_launch(
        "openai".into(),
        "http://x".into(),
        "m".into(),
        "native".into(),
        8192,
    );
    cfg.command_allowlist = default_allowlist();
    cfg.command_denylist = default_denylist();
    RulePolicy {
        workspace: PathBuf::from("/workspace"),
        command_allowlist: cfg.command_allowlist.clone(),
        command_denylist: cfg.effective_denylist(),
    }
}

/// Route a command through the engine exactly as the loop would: construct the
/// `execute_command` intent (Access::Write, `command` field set), then `check`.
fn decide(engine: &RulePolicy, command: &str) -> &'static str {
    let intent = ExecuteCommand
        .intent(&serde_json::json!({ "command": command }))
        .expect("execute_command intent construction cannot fail for a string command");
    match engine.check(&intent) {
        Decision::Allow => "allow",
        Decision::Ask => "ask",
        Decision::Deny(_) => "deny",
    }
}

struct Case {
    line: usize,
    expected: &'static str,
    command: String,
}

/// Parse the TSV. Loud on any malformed row (with the 1-based line number) so a
/// typo can never silently drop a case from coverage.
fn parse_corpus(src: &'static str) -> Vec<Case> {
    let mut cases = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = i + 1;
        // Full-line comments and blank lines are skipped.
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        // Fields are TAB-separated. Field 0 = expected, field 1 = command.
        // A 3rd+ field is an inline comment (documented asymmetry) and ignored.
        // Command spacing is preserved verbatim (whitespace-smuggling cases).
        let mut fields = raw.split('\t');
        let expected = fields
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| panic!("line {line}: missing `expected` field: {raw:?}"));
        let command = fields.next().unwrap_or_else(|| {
            panic!("line {line}: missing `command` field (need a TAB): {raw:?}")
        });
        if command.trim().is_empty() {
            panic!("line {line}: empty `command` field: {raw:?}");
        }
        if !matches!(expected, "allow" | "ask" | "deny") {
            panic!("line {line}: expected must be allow|ask|deny, got {expected:?}");
        }
        cases.push(Case {
            line,
            expected,
            command: command.to_string(),
        });
    }
    cases
}

#[test]
fn adversarial_command_corpus_matches_current_behavior() {
    let src = include_str!("policy_corpus.tsv");
    let cases = parse_corpus(src);

    // Guard against an accidentally-gutted corpus: the spec requires ≥30 rows
    // covering every closed-bypass class.
    assert!(
        cases.len() >= 30,
        "corpus shrank to {} rows (<30); every closed-bypass class must stay covered",
        cases.len()
    );

    let engine = engine();
    let mut mismatches = Vec::new();
    for case in &cases {
        let got = decide(&engine, &case.command);
        if got != case.expected {
            mismatches.push(format!(
                "  line {}: expected {}, got {} — `{}`",
                case.line, case.expected, got, case.command
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} of {} corpus rows disagree with current engine behavior:\n{}",
        mismatches.len(),
        cases.len(),
        mismatches.join("\n")
    );
}
