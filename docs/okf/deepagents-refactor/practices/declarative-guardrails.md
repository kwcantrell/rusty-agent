---
type: Practice
title: Declarative guardrails — permissions and interrupt-driven HITL
description: Steering as data, not code — first-match-wins filesystem permission rules (allow/deny/interrupt) plus per-tool interrupt configs with approve/edit/reject/respond decisions over durable interrupts.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Declarative guardrails — permissions and interrupt-driven HITL

deepagents splits steering into two declarative surfaces, both enforced in
middleware before tools run [1].

## Filesystem permissions

`FilesystemPermission` rules carry `operations` (read = ls/read_file/glob/
grep; write = write_file/edit_file/delete), glob `paths` (with `**` and
`{a,b}`), and a `mode` of `allow` | `deny` | `interrupt`. Evaluation is
**first-match-wins, top to bottom; no match means allowed** [1]. Documented
caveats worth copying into any port: only built-in fs tools are governed
(custom/MCP tools bypass), sandbox `execute` is not constrained, and
directory deletes check write permission recursively [1]. Subagents inherit
the parent's rules unless they declare their own, which replace entirely [1].

## Interrupt-driven HITL

`interrupt_on` maps tool name → config with `allowed_decisions` from
**approve** (run as-is), **edit** (modify args first), **reject** (skip,
feedback to agent), **respond** (human text becomes the tool result), plus a
`when` predicate for conditional gates (e.g. only writes outside
`/workspace/`) [1]. Interrupts surface through the graph runtime's durable
interrupt mechanism — a checkpointer is mandatory, the run pauses
indefinitely, and the caller resumes with explicit decisions on the same
thread [1]. Production guidance layers guardrail middleware on top:
model-call limits, tool-call limits, retry, fallback, and PII scrubbing [1].

## Why it matters for the refactor

The current runtime's `RulePolicy` covers adjacent ground differently: a
hard-floor command denylist, a command allowlist, workspace-boundary path
checks, and Allow/Ask/Deny decisions routed to a live `ApprovalChannel`
with Approve/ApproveAlways/Deny ([current
runtime](/perspectives/current-runtime.md)) [2]. Three deltas stand out:

- rules are code (`RulePolicy` match arms), not user-declarable data — no
  per-project glob rule lists [2][1];
- approval responses lack **edit** and **respond** — the human can gate but
  not steer arguments or stand in for a tool [2][1];
- approvals are live-channel only — there is no durable interrupt, so an
  unattended run cannot pause indefinitely and resume with decisions
  (deepagents gets this from checkpointing) [2][1].

The intent-based `ToolIntent` layer (declared access + paths + parsed
command per call) is richer than deepagents' fs-tools-only scope and worth
keeping under any port [2][1].

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
