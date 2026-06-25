# Policy Boundary Consistency — Design

**Date:** 2026-06-25
**Status:** Approved (brainstorming) → ready for plan
**Source:** Fast-follow to the merged tool-execution security boundary work
(`2026-06-24-tool-execution-security-boundary-design.md`). Findings re-verified
against current `main` on 2026-06-25; both are still live. See
`2026-06-24-security-audit-backlog.md` (Cluster C + the newly-found path bug).

## Principle

A policy decision must operate on the **same representation the action actually
executes against**. Both bugs below violate this: the runtime decides "is this
safe?" once, in a representation or at a moment that doesn't match where the
action runs. We fix both by (1) making the *decision* reuse the exact primitive
*execution* uses, and (2) re-deciding when execution changes the target.

This is the same root cause the merged boundary spec addressed for commands and
mounts; this spec closes the two remaining instances (HTTP redirects, and the
read-path approval gate).

## Finding 1 — Path decision is non-normalizing (approval-gate bypass)

**Where:** `agent/crates/agent-policy/src/engine.rs:43-50`, the `Access::Read`
branch of `RulePolicy::check`.

**Current code:**
```rust
Access::Read => {
    let all_inside = intent.paths.iter().all(|p| {
        let abs = if p.is_absolute() { p.clone() } else { self.workspace.join(p) };
        abs.starts_with(&self.workspace)
    });
    if all_inside { Decision::Allow } else { Decision::Ask }
}
```

**Bug:** `intent.paths` carries the raw, un-normalized, model-supplied path
(e.g. `agent/crates/agent-tools/src/fs/read.rs:23-24` pushes `path.clone()`
verbatim). `Path::starts_with` is component-wise, so a relative
`../../etc/passwd` joined to `/work` becomes `/work/../../etc/passwd`, which
**does** `starts_with("/work")` → `Decision::Allow`. That short-circuits the
approval prompt at `agent-core/src/loop_.rs:273-274`.

Execution disagrees: `execute()` resolves through
`agent-tools`'s `resolve_in_workspace` (`agent/crates/agent-tools/src/fs/paths.rs:15`),
which lexically collapses `.`/`..` **before** the boundary check and hard-denies
the escape. So today the file is not actually read — but the **approval gate is
bypassed** for escaping reads, and any read tool that resolved paths even
slightly differently would turn this into a real leak. The two guards must not
disagree.

**Severity:** Medium (approval-gate bypass; not arbitrary read today because
`execute()` still blocks via `resolve_in_workspace`).

### Fix

Decide "inside workspace?" by calling the function execution already uses, so the
two cannot drift:

```rust
use agent_tools::fs::resolve_in_workspace; // already pub-exported

Access::Read => {
    let all_inside = intent.paths.iter().all(|p| {
        resolve_in_workspace(&self.workspace, &p.to_string_lossy()).is_ok()
    });
    if all_inside { Decision::Allow } else { Decision::Ask }
}
```

- An escaping read now returns `Decision::Ask` (prompts the user), matching the
  existing `read_outside_workspace_asks` semantics — the model already intends
  outside-workspace reads to require approval.
- `resolve_in_workspace` is **already** reachable as
  `agent_tools::fs::resolve_in_workspace` (`fs/mod.rs:4` re-exports it; `fs` is a
  `pub mod`), so no export change is required. A top-level
  `pub use fs::resolve_in_workspace` in `agent-tools/src/lib.rs` is optional
  polish only. `agent-policy` already depends on `agent-tools` (it imports
  `Access`, `Display`, `ToolIntent`), so no new dependency and no dependency
  cycle.
- The lexical-only / symlink limitation documented on `resolve_in_workspace` is
  now inherited **identically** by the decision and the execution — deliberately,
  since they must match. Symlink-escape hardening stays deferred to OS-level
  sandboxing (see `docs/superpowers/context/os-sandboxing.md`); it is explicitly
  out of scope here.

## Finding 2 — HTTP redirects bypass the host policy

**Where:** `agent/crates/agent-http/src/tool.rs:153-230`, the `execute()`
redirect loop.

**Bug:** The host approval decision is computed **once**, from the original URL,
in `intent()` (`tool.rs:131-147`, `self.policy.decide(host)`), and that drives
the approval gate before `execute()` runs. The redirect loop then re-resolves and
re-runs **only** the SSRF IP guard (`self.guard.check`, `tool.rs:167-171`) on each
hop — it never re-runs `self.policy.decide()`. So an approved fetch to an
allowlisted host can `302` to any other public, non-SSRF-blocked host and the
fetch proceeds with no fresh allow/ask decision. The only redirect-target checks
today are: SSRF IP ranges, the `MAX_REDIRECTS` cap, and an `http`/`https` scheme
check.

**Severity:** High.

### Fix — re-decide host policy on every hop (deny unapproved hosts)

Chosen approach (of three considered — see Alternatives): re-run the host policy
per hop and **deny** a redirect to a host that was neither allowlisted nor part
of the original approval. No mid-execute interactive prompt; this mirrors the
per-hop SSRF re-validation already present and keeps the existing
"decide-before-execute" model intact.

Before the loop, capture the approved host:
```rust
let approved_host = url.host_str()
    .ok_or_else(|| ToolError::InvalidArgs("url has no host".into()))?
    .to_ascii_lowercase();
```

In the redirect branch, after the new `url` is computed and the existing scheme
check passes, gate the new host before `continue`:
```rust
let new_host = url.host_str()
    .ok_or_else(|| ToolError::InvalidArgs("redirect url has no host".into()))?;
let ok = matches!(self.policy.decide(new_host), HostDecision::Allow)
      || new_host.eq_ignore_ascii_case(&approved_host);
if !ok {
    return Err(ToolError::Denied(format!(
        "redirect from {approved_host} to un-approved host {new_host}; \
         fetch it directly to approve"
    )));
}
```

A hop is allowed iff the target host is **allowlisted** (`HostDecision::Allow`)
**or** is the **same host already approved** (covers `http→https` upgrades and
path-only redirects, where the host is unchanged but `decide()` might still be
`Ask` for a user-approved host). The SSRF guard continues to run at the top of
the loop for every hop, unchanged. No new plumbing: `self.policy` and the loop
already exist.

## Error handling

- **Redirect to an un-approved host:** return `ToolError::Denied` with a message
  naming both the approved origin host and the blocked redirect target. The
  response body for that hop is **not** read.
- **Path:** no new error path. An escaping read routes to `Decision::Ask`; the
  approval channel then prompts, exactly as it already does for any
  outside-workspace read.

## Testing (TDD — write the failing test first)

**`agent-policy/src/engine.rs`:**
- `../../etc/passwd` (relative) → `Decision::Ask`. *(Regression test that
  currently FAILS — proves the bypass; today returns `Allow`.)*
- `/work/../etc` (absolute escape) → `Decision::Ask`.
- `/work/sub/../a.txt` (traverses but stays inside) → `Decision::Allow`.
- All existing `engine.rs` tests stay green
  (`read_inside_workspace_allowed`, `read_outside_workspace_asks`,
  `write_always_asks`, command-floor tests).

**`agent-http/src/tool.rs`:**
- Redirect to an **allowlisted** host → follows and fetches the final body.
- Redirect to a **non-allowlisted, different** host → `ToolError::Denied`
  (the new regression test; today it wrongly succeeds).
- Same-host `http→https` (or path-only) redirect → follows.
- Redirect whose target IP is SSRF-blocked → still `Denied` (existing behavior,
  guard against regression).
- Reuse the existing `follows_redirect_then_fetches` mock-server harness;
  construct the tool with a `NetworkPolicy` whose allowlist contains the
  origin host but not the redirect target.

## Scope

**In scope:** `agent-policy/src/engine.rs` (Read branch) and
`agent-http/src/tool.rs` (redirect loop), plus tests in both crates.
`resolve_in_workspace` is already public, so no `agent-tools` change is required
(an optional top-level re-export aside).

**Out of scope (explicit non-goals):**
- Mid-execute interactive approval (threading `ApprovalChannel` through
  `ToolCtx`) — heavier, inverts the decide-before-execute model.
- Symlink / `canonicalize` hardening — deferred to OS sandboxing; the lexical
  guard's limitation is intentionally shared by both decision and execution.
- Audit Clusters A (server concurrency & leaks) and B (loop robustness) — tracked
  separately in `2026-06-24-security-audit-backlog.md`.

## Alternatives considered (redirect strategy)

1. **Deny unapproved-host redirects, re-decide per hop — CHOSEN.** Minimal,
   safe, mirrors existing per-hop SSRF re-validation, no architectural change.
   Cost: a redirect to a non-allowlisted host fails rather than prompting; the
   user re-issues the fetch against the final URL to approve it.
2. **Mid-execute interactive approval.** Thread `ApprovalChannel` into `ToolCtx`
   so `execute()` can prompt per hop. More flexible but touches `ToolCtx`, the
   loop's approval flow, and every tool — disproportionate for a fast-follow, and
   inverts the current model.
3. **Same-host redirects only.** Strictest and simplest, but breaks legitimate
   allowlisted cross-host redirects (e.g. apex→www, redirect to a CDN).
