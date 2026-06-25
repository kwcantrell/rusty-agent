# Policy Boundary Consistency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the two remaining "decide once, execute elsewhere" policy gaps — the HTTP redirect loop never re-checks the host policy, and the read-path approval gate uses a non-normalizing path check that lets `../`-escaping reads skip approval.

**Architecture:** Two independent, single-file fixes that each make a *policy decision* reuse the exact primitive *execution* already uses. (1) `RulePolicy::check`'s Read branch decides "inside workspace?" via `agent_tools::fs::resolve_in_workspace` (the same resolver `execute()` uses), so the two can't drift. (2) `FetchUrl::execute`'s redirect loop re-runs `NetworkPolicy::decide` on every hop and denies a redirect to a host that is neither allowlisted nor the already-approved origin host.

**Tech Stack:** Rust, `tokio`, `reqwest`, `wiremock` (test HTTP), `url`. Crates: `agent-policy`, `agent-http`, depending on `agent-tools`.

## Global Constraints

- TDD: write the failing test first, watch it fail, then the minimal fix.
- Reuse existing primitives — do **not** add new normalization or policy code. Use `agent_tools::fs::resolve_in_workspace` (already `pub`) and `NetworkPolicy::decide` (already in `execute()` via `self.policy`).
- No new crate dependencies: `agent-policy` already depends on `agent-tools`; `agent-http` already imports both `NetworkPolicy` and `HostDecision`.
- Preserve existing behavior: every existing test in `agent-policy/src/engine.rs` and `agent-http/src/tool.rs` must stay green.
- Run tests from the workspace root with `cargo` on PATH (`source ~/.cargo/env` first if needed).
- Out of scope (do not touch): mid-execute interactive approval, symlink/`canonicalize` hardening, audit Clusters A and B.

---

### Task 1: Normalize the read-path approval gate (`agent-policy`)

**Files:**
- Modify: `agent/crates/agent-policy/src/engine.rs:43-50` (the `Access::Read` branch of `RulePolicy::check`) and the `use` line at top
- Test: `agent/crates/agent-policy/src/engine.rs` (the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `agent_tools::fs::resolve_in_workspace(workspace: &Path, arg: &str) -> Result<PathBuf, ToolError>` — returns `Ok` iff `arg`, resolved against `workspace` with lexical `.`/`..` collapse, stays inside `workspace`; `Err(ToolError::Denied(_))` on escape. Already `pub` at `agent_tools::fs::resolve_in_workspace`.
- Produces: no new public surface; `RulePolicy::check` behavior unchanged except an escaping read now returns `Decision::Ask` instead of `Decision::Allow`.

- [ ] **Step 1: Write the failing test**

Add these three tests inside the existing `mod tests` in `agent/crates/agent-policy/src/engine.rs` (after `read_outside_workspace_asks`). The `policy()` and `intent()` helpers already exist there.

```rust
#[test]
fn read_relative_dotdot_escape_asks() {
    // ../../etc/passwd joined to /work escapes; must require approval, not Allow.
    assert!(matches!(
        policy().check(&intent(Access::Read, vec!["../../etc/passwd"], None)),
        Decision::Ask
    ));
}

#[test]
fn read_absolute_dotdot_escape_asks() {
    // /work/../etc normalizes to /etc — outside the workspace.
    assert!(matches!(
        policy().check(&intent(Access::Read, vec!["/work/../etc/x"], None)),
        Decision::Ask
    ));
}

#[test]
fn read_dotdot_staying_inside_allows() {
    // sub/../a.txt normalizes to /work/a.txt — still inside.
    assert!(matches!(
        policy().check(&intent(Access::Read, vec!["sub/../a.txt"], None)),
        Decision::Allow
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agent-policy read_relative_dotdot_escape_asks read_absolute_dotdot_escape_asks read_dotdot_staying_inside_allows`
Expected: `read_relative_dotdot_escape_asks` and `read_absolute_dotdot_escape_asks` FAIL (current code returns `Allow` for the `..`-escape because `starts_with` is non-normalizing). `read_dotdot_staying_inside_allows` may already pass. The two escape tests failing is the proof of the bug.

- [ ] **Step 3: Write the minimal implementation**

In `agent/crates/agent-policy/src/engine.rs`, add the import near the existing `use agent_tools::...` line at the top of the file:

```rust
use agent_tools::fs::resolve_in_workspace;
```

Replace the `Access::Read` arm (currently lines ~44-50) with:

```rust
Access::Read => {
    // Decide "inside workspace?" with the SAME resolver execute() uses, so the
    // approval gate and the execution guard can never disagree (resolve_in_workspace
    // collapses `.`/`..` before the boundary check). An escaping read -> Ask.
    let all_inside = intent.paths.iter().all(|p| {
        resolve_in_workspace(&self.workspace, &p.to_string_lossy()).is_ok()
    });
    if all_inside { Decision::Allow } else { Decision::Ask }
}
```

Leave the `Access::Write => Decision::Ask` arm and the command branch unchanged.

- [ ] **Step 4: Run the full crate test suite to verify pass + no regressions**

Run: `cargo test -p agent-policy`
Expected: PASS — the three new tests pass, and all existing tests (`read_inside_workspace_allowed`, `read_outside_workspace_asks`, `write_always_asks`, the command-floor/metachar tests) stay green.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-policy/src/engine.rs
git commit -m "fix(policy): normalize read-path approval gate via resolve_in_workspace

The Read branch used a non-normalizing workspace.join(p).starts_with(workspace),
so a ../-escaping relative path returned Decision::Allow and skipped approval.
Decide via the same resolver execute() uses; escaping reads now route to Ask."
```

---

### Task 2: Re-decide host policy on every redirect hop (`agent-http`)

**Files:**
- Modify: `agent/crates/agent-http/src/tool.rs:149-231` (`FetchUrl::execute`: capture the approved host before the loop; gate the new host in the redirect branch)
- Test: `agent/crates/agent-http/src/tool.rs` (the existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `self.policy: NetworkPolicy` with `decide(&self, host: &str) -> HostDecision` (`HostDecision::Allow | Ask`), already a field of `FetchUrl` and already imported. `FetchUrl::with_guard(policy: NetworkPolicy, guard: SsrfGuard)` (test-only ctor) and `SsrfGuard::allow_all()` already exist.
- Produces: redirect behavior change only — a hop to a host that is neither `HostDecision::Allow` nor equal (case-insensitive) to the original approved host now returns `ToolError::Denied`. No signature changes.

- [ ] **Step 1: Write the failing tests**

Add these two tests inside the existing `mod tests` in `agent/crates/agent-http/src/tool.rs` (next to `follows_redirect_then_fetches`). The `ctx()` and `permissive()` helpers and the `wiremock` imports already exist there.

```rust
#[tokio::test]
async fn redirect_to_unapproved_host_is_denied() {
    // Origin 127.0.0.1 (server) redirects to a different host not in the allowlist.
    // The host check fires BEFORE the next hop's DNS resolution, so no network needed.
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/from"))
        .respond_with(ResponseTemplate::new(302)
            .insert_header("location", "http://blocked.invalid/to"))
        .mount(&server).await;

    let url = format!("{}/from", server.uri());
    // permissive() has an EMPTY allowlist -> every host is Ask.
    let err = permissive().execute(json!({ "url": url }), &ctx()).await.unwrap_err();
    match &err {
        ToolError::Denied(msg) => assert!(
            msg.contains("un-approved host") && msg.contains("blocked.invalid"),
            "expected un-approved-host denial, got: {msg}"
        ),
        other => panic!("expected Denied(un-approved host), got {other:?}"),
    }
}

#[tokio::test]
async fn redirect_to_allowlisted_host_passes_policy_gate() {
    // Same different-host redirect, but the target IS allowlisted: the policy gate
    // must let it through. allowed.invalid never resolves (RFC 6761 reserved TLD),
    // so the hop proceeds past the gate and fails at DNS with NotFound — NOT a
    // policy Denied. That distinguishes "gate passed" from "gate blocked".
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/from"))
        .respond_with(ResponseTemplate::new(302)
            .insert_header("location", "http://allowed.invalid/to"))
        .mount(&server).await;

    let t = FetchUrl::with_guard(
        NetworkPolicy::new(&["allowed.invalid".to_string()]),
        SsrfGuard::allow_all(),
    );
    let url = format!("{}/from", server.uri());
    let err = t.execute(json!({ "url": url }), &ctx()).await.unwrap_err();
    assert!(
        matches!(err, ToolError::NotFound(_)),
        "allowlisted host should pass the policy gate and fail later at DNS (NotFound), got {err:?}"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p agent-http redirect_to_unapproved_host_is_denied redirect_to_allowlisted_host_passes_policy_gate`
Expected: `redirect_to_unapproved_host_is_denied` FAILS — current code follows the cross-host redirect (no policy re-check), so it does not return `Denied(un-approved host)` (it proceeds to resolve `blocked.invalid` → `NotFound`). This failure is the proof of the bypass. `redirect_to_allowlisted_host_passes_policy_gate` likely already passes (current code lets all redirects through), which is fine — it pins the allow arm so the fix doesn't over-deny.

- [ ] **Step 3: Write the minimal implementation**

In `agent/crates/agent-http/src/tool.rs`, in `FetchUrl::execute`, capture the approved host immediately after `let mut url = parse_url(&args)?;` (line ~150):

```rust
let mut url = parse_url(&args)?;
// The host the caller's approval was granted for. Every redirect hop must land on
// this host or an allowlisted one; anything else needs a fresh approval the loop
// can't request mid-execute, so we deny it.
let approved_host = url
    .host_str()
    .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?
    .to_ascii_lowercase();
let mut hops = 0usize;
```

Then, inside the redirect branch, **after the existing `http`/`https` scheme check and before `continue`** (currently around lines 203-206), insert the host re-decision:

```rust
                    if !matches!(url.scheme(), "http" | "https") {
                        return Err(ToolError::Denied(format!("redirect to non-http scheme '{}'", url.scheme())));
                    }
                    // Re-run the host policy for the new target (decide-at-execution).
                    let new_host = url
                        .host_str()
                        .ok_or_else(|| ToolError::InvalidArgs("redirect url has no host".into()))?;
                    let host_ok = matches!(self.policy.decide(new_host), HostDecision::Allow)
                        || new_host.eq_ignore_ascii_case(&approved_host);
                    if !host_ok {
                        return Err(ToolError::Denied(format!(
                            "redirect from {approved_host} to un-approved host {new_host}; \
                             fetch it directly to approve"
                        )));
                    }
                    continue; // re-resolve + re-validate the new target
```

Do not move the scheme check — it must stay before the host check so a `file://` redirect (no host) still returns the scheme `Denied` rather than an `InvalidArgs`.

- [ ] **Step 4: Run the full crate test suite to verify pass + no regressions**

Run: `cargo test -p agent-http`
Expected: PASS — both new tests pass, and every existing test stays green, specifically: `follows_redirect_then_fetches` (same-host `/from→/to`: `new_host == approved_host` → allowed), `redirect_to_non_http_scheme_is_denied` (scheme check fires first), `too_many_redirects_is_failed` (hop cap trips before the host check), `fetches_html_and_returns_readable_text`, `binary_content_is_refused`, `strict_guard_blocks_loopback_target`.

- [ ] **Step 5: Commit**

```bash
git add agent/crates/agent-http/src/tool.rs
git commit -m "fix(http): re-decide host policy on every redirect hop

The redirect loop re-ran only the SSRF guard, never policy.decide(), so an
approved fetch to an allowlisted host could 302 to any public host. Capture the
approved origin host and deny a hop to any host that is neither allowlisted nor
the same approved host."
```

---

### Task 3: Final verification

**Files:** none (verification only)

- [ ] **Step 1: Build and test both crates plus dependents**

Run: `cargo test -p agent-policy -p agent-http`
Expected: PASS, no warnings about unused imports.

- [ ] **Step 2: Workspace-wide sanity check**

Run: `cargo build` then `cargo test` (or at minimum `cargo test -p agent-core -p agent-tools` to confirm no downstream consumer of `RulePolicy`/`FetchUrl` broke).
Expected: PASS — the only behavioral change is escaping reads → `Ask` and cross-host redirects → `Denied`; no signatures changed.

- [ ] **Step 3: Confirm the spec's testing checklist is satisfied**

Cross-check against `docs/superpowers/specs/2026-06-25-policy-boundary-consistency-design.md` → "Testing" section: every listed case has a corresponding passing test (engine: relative escape / absolute escape / inside-traversal / existing green; http: allowlisted-follows / unapproved-denied / same-host-follows / SSRF-blocked-still-denied). If any is missing, add it before finishing.
