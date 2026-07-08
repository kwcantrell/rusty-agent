---
type: Perspective
title: Current runtime architecture (rusty-agent)
description: Capability-by-capability snapshot of the agent/ workspace as it stands — what already matches deepagents, through which traits, and where the seams are.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Current runtime architecture (rusty-agent)

Snapshot of the `agent/` Cargo workspace (verified against live source
2026-07-08) [1]. File references are live-source anchors; re-read before
editing — this page describes the refactor's *starting point*.

## The loop and its seams

`AgentLoop` (`agent/crates/agent-core/src/loop_.rs`) drives a streamed
tool-calling loop: memory retrieval → context build → completion with retry →
protocol parse → parallel tool execution (bounded, default 8) → policy/approval
per call → results appended → repeat until stop/max-turns/cancel [1]. Built-in
loop guards: stuck detection nudges on the 3rd identical tool-call turn and
aborts on the 5th, and malformed tool calls get one built-in re-ask retry
(per malformed call on the native protocol, per turn on the prompted one)
before failure is terminal [1]. It
already exposes trait seams that a middleware-style decomposition can attach
to [1]:

- `ModelClient` (openai-compat + claude-cli backends), `ToolCallProtocol`
  (native vs prompted JSON)
- `Tool` + `ToolRegistry` (with `description_overrides` as an eval seam)
- `PolicyEngine` + `ApprovalChannel` (Allow/Ask/Deny; CLI and daemon frontends)
- `SandboxStrategy` (host vs Docker with degraded-posture refusal)
- `EventSink` (every token, tool event, usage, curation event)
- `ContextManager` (implemented by `CuratedContext`), `OffloadStore`,
  `Retriever`

Assembly is centralized in `assemble_loop()`
(`agent/crates/agent-runtime-config/src/assemble.rs`): registry build → skills
→ memory → MCP → dispatch tool → system-prompt composition → routed
subagent/compaction models → `AgentLoop` [1].

## What already matches the deep-agents shape

- **Sub-agents** — `DispatchAgentTool`
  (`agent/crates/agent-core/src/dispatch.rs`): isolated child loop, fresh
  context, snapshot of parent tools, depth/turn/timeout limits, routed child
  model, event forwarding with `parent_id`; per-call args are `prompt`,
  an optional `tools` allowlist, and an optional `role` preamble appended to
  the child's system prompt [1].
- **Context engineering** — `CuratedContext`: pinned system + goal
  re-grounding block + folded-facts ledger + compaction summary + windowed
  history; large tool results offload to an `OffloadStore` side table with a
  `context_recall` tool; compaction at a high-water threshold (default 85%)
  on a routed compaction model [1].
- **Skills** — SKILL.md-style markdown with YAML frontmatter, discovery dirs,
  `ListSkills`/`UseSkill` load-on-demand plus presets inlined into the system
  prompt [1].
- **Long-term memory** — `agent-memory`: remember/recall/forget tools over
  SQLite (plain rusqlite; similarity is in-process cosine over ONNX/fastembed
  embeddings, no sqlite-vec extension); auto-recall injected at run start
  under a token budget (default 512 tokens) [1].
- **Permissions & HITL** — `RulePolicy`: hard-floor denylist, command
  allowlist, workspace-boundary path checks; `Ask` routes to a human approval
  channel with Approve/ApproveAlways/Deny [1].
- **Sandboxed execution** — Docker sandbox with resource limits, network
  off by default, and refusal (not silent host fallback) when degraded [1].

## What is absent or shaped differently

- **No planning/todo tool** — no plan state anywhere in the core; planning is
  implicit in prompts/skills, with no audit trail of plan vs outcome [1].
- **No virtual filesystem** — file tools operate directly on the real
  workspace; there is no backend abstraction (state/store/disk/composite) and
  offloaded results live in a separate id-keyed side table rather than an
  agent-visible filesystem [1].
- **No middleware abstraction** — cross-cutting behavior lives inside
  `AgentLoop` and `assemble_loop()` rather than composable units; the seams
  above are swap-points, not a stack [1].
- **Single sub-agent shape** — one general dispatch tool with per-call
  `role`/`tools` customization, but no persistent named sub-agent registry
  with per-subagent prompt/tools/model specs [1].

## Coupling hotspots a refactor must respect

- Loop and context manager are tightly bound: `CuratedContext` maintenance
  (compaction) is loop-resident [1].
- Registry and `LoopConfig` freeze at assembly; no mid-session tool or config
  changes (skills work around this via load-on-demand) [1].
- Everything converges on `EventSink`'s wire format — frontends depend on
  it [1].
- God nodes by graph degree: `policy()`, `registry()`, `assemble_loop()`,
  `CuratedContext`, server `Session` [1].

# Citations

1. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
