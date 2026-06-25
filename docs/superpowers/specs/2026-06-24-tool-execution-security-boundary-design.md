# Tool-Execution Security Boundary — Design

**Date:** 2026-06-24
**Status:** Approved for planning
**Scope:** `agent-policy`, `agent-sandbox`, `agent-tools`

## Problem

This runtime exists to safely execute model-driven and MCP-driven tool calls. Its
defense-in-depth layers (command policy, sandbox, approval gating) are documented as a
floor that must hold *even against the model or an injected settings frame*
(`agent-runtime-config/src/runtime_config.rs:5-8`). A graphify-guided audit found that
floor is, in several places, cosmetic.

The findings share one root cause:

> **The runtime decides "is this safe?" once, in a representation or location that does
> not match where the action actually executes.**

- The command policy matches **substrings of a raw string** that `sh -c` will later
  re-parse and re-interpret.
- The sandbox mount guard blocks a **tiny hardcoded list** that omits credential
  directories, so the `--read-only` boundary can be widened to sensitive host paths.
- The git tool **shells out on the host**, bypassing the sandbox that the shell tool
  goes through.

This spec fixes all three at the real execution boundary. A fourth instance of the same
root cause — HTTP redirects bypassing the approval policy — is explicitly deferred to a
fast-follow spec (see *Out of Scope*).

## Background: current behavior

### Command policy (`agent-policy/src/engine.rs:30-46`)

```rust
if self.command_denylist.iter().any(|d| cmd.contains(d.as_str())) {
    return Decision::Deny(...);
}
const SHELL_META: &[char] = &[';', '&', '|', '`', '$', '(', ')', '<', '>', '\n', '\\'];
let first = cmd.split_whitespace().next().unwrap_or("");
let has_meta = cmd.contains(|c: char| SHELL_META.contains(&c));
if !has_meta && self.command_allowlist.iter().any(|a| a == first) {
    return Decision::Allow;
}
return Decision::Ask;
```

The hard-floor denylist (e.g. `sudo`, `rm -rf /`, `dd if=`, `:(){`) is matched as a raw
substring; the auto-allow gate keys off the first whitespace token plus an incomplete
metacharacter set.

**Verified bypasses:**

- `rm -fr /`, `rm --recursive --force /`, `rm -rf  /` (two spaces) — none equal the
  substring `rm -rf /`, so the floor misses them.
- `cat {a,b}`, `ls *`, `cat ~/x` — `SHELL_META` omits `{`, `}`, `*`, `?`, `~`, `!`, `#`,
  and quotes, so these auto-allow and are then expanded by `sh -c`.
- A planted-path program is partially mitigated only because the first token must equal
  an allowlist entry exactly; `/usr/bin/sudo` is *not* on the allowlist but is caught by
  the `sudo` substring — incidental, not by design.

### Sandbox mount validation (`agent-sandbox/src/mounts.rs:4-35`)

`validate_mount` canonicalizes the path and rejects only `/`, `$HOME` *root*, the docker
socket, and `/run`/`/var/run`. It does **not** reject `~/.ssh`, `~/.aws`, `~/.gnupg`,
`/etc`, `/usr`, etc. The returned canonical path is later interpolated into a
`docker -v src:dst:mode` argument, so a path containing `:` (legal on Linux) corrupts the
`-v` parse.

### Git tool (`agent-tools/src/git.rs:5-8`)

```rust
let out = tokio::process::Command::new("git")
    .args(args).current_dir(&ctx.workspace).output().await ...
```

`git_status` / `git_diff` / `git_commit` run `git` directly on the host. `ToolCtx`
already carries `sandbox: Arc<dyn ...>` (the executor `ExecuteCommand` uses), but the git
helper ignores it. Under `enforce` mode the shell tool is containerized and git is not,
so workspace-controlled `.git/hooks` (e.g. a malicious `pre-commit`) execute on the host.

## Goals

1. The command policy's allow/deny decisions are robust against whitespace, flag
   reordering/bundling, path prefixes, and shell metacharacters.
2. The sandbox writable boundary cannot be widened to system or credential paths through
   `extra_rw` / `extra_ro` config, and mount construction is injection-safe.
3. The git tool honors the same sandbox boundary as the shell tool.
4. Every verified bypass from the audit becomes a regression test.

## Non-Goals

- Per-program argument policies (e.g. constraining which `git`/`cargo` subcommands or
  flags are allowed). Allowlisting a program trusts its flags; see *Known residual risk*.
- Replacing the approval UX or the allow/deny config schema.
- HTTP redirect re-validation (deferred — *Out of Scope*).
- Any change to `agent-core`, `agent-memory`, or the memory auto-retrieval feature.

## Design

### Component 1 — Parse-then-classify command policy (`agent-policy`)

Replace raw-string matching with a model that mirrors how the command will actually run.

**Tokenization.** A command string is split into one or more *simple commands* separated
by shell control operators (`;`, `&&`, `||`, `|`, `&`, newline), respecting quoting. Each
simple command is parsed into an argv vector. Quote-aware splitting uses the `shell-words`
crate; operator detection uses a small scanner that recognizes the control operators
*outside* quotes (`shell-words` itself treats `&&` as a literal token, so operator
detection is separate from argv splitting).

**Decision order:**

1. **Hard-floor deny** (cannot be approved away). Two complementary layers; deny if
   **either** fires:
   - **Layer A — structural**, per simple command. Deny if any simple command matches:
     - program *basename* ∈ {`sudo`, `doas`, `su`}  (basename match catches `/usr/bin/sudo`)
     - program basename `rm` with a recursive flag — `-r`, `-R`, `--recursive`, or any
       bundled short flag containing `r` (e.g. `-rf`, `-fr`) — **and** a root target (`/`,
       `/*`, or `--no-preserve-root` present)
     - program basename `dd` with an `of=` argument naming a block device (`/dev/...`)
     - the fork-bomb token sequence `:(){`
   - **Layer B — always-on substring backstop.** Independent of Layer A (not only on
     parse failure), match each denylist pattern against a whitespace-normalized form of
     the command. This catches no-space operators (`echo x&&sudo`), unparseable strings
     (unbalanced quotes), and configured denylist literals. It is fail-safe and preserves
     today's behavior (`cmd.contains(pattern)`), only normalized so `rm -rf  /` with extra
     spaces still matches `rm -rf /`.

   Layer A upgrades detection of flag/spacing *variants* of known-dangerous programs that
   substring matching cannot see (`rm -fr /`, `rm --recursive --force /`); Layer B retains
   coverage of operator-hidden and unparseable cases. The known cost of Layer B is the
   pre-existing false-positive property (a benign mention of a denylisted literal is
   denied) — acceptable for a conservative floor and unchanged from current behavior.

2. **Auto-allow** (deny-by-default posture). Allow **only if all** hold:
   - the command tokenized cleanly,
   - it is a **single** simple command (no control operators),
   - it contains no redirections, command substitutions (`$(...)`, backticks), globs
     (`*`, `?`, `[`), brace expansion (`{`), tilde (`~`), or process substitution,
   - the program token contains no `/` (reject explicit-path invocations from the
     auto-allow path so a planted `./ls` is not auto-allowed),
   - the program token ∈ `command_allowlist`.

3. **Ask** — everything else.

**Bypasses this closes:** `rm -fr /`, `rm --recursive --force /`, `rm -rf  /`,
`echo x && sudo reboot` (floor sees the second simple command), `cat {a,b}`, `ls *`,
`cat ~/x` (glob/brace/tilde → not auto-allowed → Ask).

**Interface impact.** `RulePolicy` keeps its `command_allowlist` / `command_denylist`
fields and `PolicyEngine::check` signature. `command_denylist` continues to feed the
floor: entries are interpreted as structural rules where they parse to a known dangerous
shape, and as normalized-substring backstop patterns otherwise — so existing config and
the `HARD_FLOOR_DENYLIST` keep working, only more strictly. No schema change.

**Known residual risk (documented, not fixed here):** allowlisting a program trusts all
of its flags — `git -c core.sshCommand=… <cmd>` remains possible because `git` is on the
allowlist. Mitigation is keeping the allowlist minimal; per-program argument rules are a
future extension (a Non-Goal here).

### Component 2 — Deny-by-default mount guard (`agent-sandbox`)

Replace the small blocklist in `validate_mount` with a sensitive-path guard.

**Rule.** After canonicalization, reject the mount if the canonical path **equals or is a
descendant of** any sensitive root:

- System roots: `/etc`, `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/boot`, `/sys`,
  `/proc`, `/dev`, `/var/lib/docker`, `/run`, `/var/run`, the docker socket, and `/`
  itself.
- HOME root itself, plus HOME credential subdirectories (and their descendants): `.ssh`,
  `.aws`, `.gnupg`, `.kube`, `.docker`, `.config/gcloud`, `.netrc`, `.git-credentials`.
  Ordinary project directories elsewhere under HOME remain allowed.

The "equals or descendant" check means mounting `/etc/foo` is rejected because `/etc` is a
sensitive root. HOME is handled by listing the specific credential subdirs as roots rather
than treating all of HOME as a prefix (which would block legitimate project dirs).

**Injection-safe construction.** Reject any canonical path containing `:` in
`validate_mount` (it corrupts the `docker -v src:dst:mode` argument, where source == target
== the host path). This is the minimal robust fix; switching the construction to
`docker --mount type=bind,source=…,target=…` is an acceptable alternative but is not
required by this spec.

**Behavior preserved.** Existing accepted cases (a real subdir under HOME, `~/sub`
expansion, symlinked-HOME rejection) continue to pass.

### Component 3 — Git tool through the sandbox (`agent-tools`)

Route the `git()` helper through `ctx.sandbox` — the same `Arc<dyn ...>` executor that
`ExecuteCommand` uses — instead of `tokio::process::Command::new("git")`. The argv
(`["git", ...subcommand]`) and `ctx.workspace` working directory are passed to the
executor.

- Under the default `HostExecutor` (and in tests) git still runs on the host: behavior is
  unchanged for the non-enforce path.
- Under `enforce` mode git runs inside the container, so workspace-controlled `.git/hooks`
  execute in the sandbox rather than on the host — closing the boundary inconsistency.

This requires the sandbox image to contain `git` and to mount the workspace, both of
which already hold for the shell tool in enforce mode.

## Testing

TDD, matching the repository's existing dense unit-test style. Each audit bypass becomes a
regression test.

**Policy (`agent-policy`):**
- Floor denies: `rm -rf /`, `rm -fr /`, `rm --recursive --force /`, `rm -rf  /` (double
  space), `/usr/bin/sudo reboot`, `echo hi && sudo reboot`, `dd if=/dev/zero of=/dev/sda`,
  `:(){ :|:& };:`.
- Floor parse-failure backstop: an unbalanced-quote string containing `sudo` denies or
  asks (never allows).
- Auto-allow rejects: `cat {a,b}`, `ls *`, `cat ~/x`, `ls | sh`, `cat x; curl evil`,
  `./ls` (explicit path), `ls && curl evil`.
- Auto-allow accepts: `ls -la`, `git status`, `cat file.txt` (single simple command,
  allowlisted, no metacharacters).
- Non-command intents (read/write path boundary) behave exactly as today.

**Mounts (`agent-sandbox`):**
- Reject: `~/.ssh`, `~/.aws/credentials`, `/etc`, `/etc/passwd`, `/usr/bin`,
  `/var/lib/docker`, a path containing `:`.
- Accept: a real project subdir under HOME, `~/sub` expansion.
- Preserve existing rejections: `/`, HOME root, docker socket, symlinked HOME root.

**Git (`agent-tools`):**
- A recording fake `SandboxExecutor` asserts `git status`/`git_commit` dispatch the
  expected argv **through the sandbox** rather than spawning on the host.
- Existing host-path git tests (`git_status_reports_untracked`,
  `git_commit_commits_staged_changes`) continue to pass under `HostExecutor`.

## Out of Scope (fast-follow, same root cause)

**HTTP redirect re-validation** (`agent-http/src/tool.rs:190-207`). The approval/allowlist
decision is computed from the *original* URL's host; the redirect loop re-runs only the
SSRF guard, never `policy.decide()`, so an approved fetch to an allowlisted host can 302 to
any public host. This is the same "decide once, execute elsewhere" root cause and should
be a separate small spec: re-run the host policy decision on every redirect hop.

## Risks & Mitigations

- **Stricter policy may surface new approval prompts** for commands that previously
  auto-allowed via the loose first-token check. This is the intended deny-by-default
  posture; the allowlist remains user-configurable.
- **`shell-words` dependency.** A small, well-established crate; isolated to
  `agent-policy`. Operator scanning is implemented locally to avoid pulling a full shell
  parser.
- **Mount guard could reject a legitimate path** under a sensitive root. Acceptable: such
  mounts are exactly what the boundary is meant to prevent; users can mount a non-sensitive
  parent or a project-local copy.
- **Git-through-sandbox requires git in the image.** Already required for shell commands
  in enforce mode; documented as a precondition.
