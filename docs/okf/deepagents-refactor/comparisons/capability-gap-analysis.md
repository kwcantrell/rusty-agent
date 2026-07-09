---
type: Comparison
title: Capability gap analysis — deepagents vs rusty-agent
description: Capability-by-capability verdict — present / partial / absent / current-is-ahead — mapping every deepagents pillar onto the existing crates, with the concrete deltas a refactor must close.
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Capability gap analysis — deepagents vs rusty-agent

Each row maps a deepagents capability onto the current `agent/` workspace.
Per-row provenance: every deepagents cell traces to [1] and [2]; every
current-runtime cell traces to [3] and is expanded, with file anchors, in
[the current-runtime perspective](/perspectives/current-runtime.md).
Verdicts: **ahead** (current design exceeds the target), **match** (shape
already right), **partial** (exists, wrong shape), **absent**,
**unassessed** (not investigated this pass).

| Capability | deepagents | Current runtime | Verdict |
|---|---|---|---|
| Middleware composition | Ordered stack, hooks + tools + state per unit | Trait seams, but cross-cutting logic inside `AgentLoop`/`assemble_loop()` | **absent** (seams exist) |
| Planning tool | `write_todos` no-op + prompt discipline | Nothing | **absent** |
| Virtual filesystem | 10 tools over `BackendProtocol`, composite prefix routing | Direct host-workspace file tools | **absent** |
| Result offloading | Evict >20k-token results to files, pointer + preview, recover via read/grep | `OffloadStore` side table + bespoke `context_recall` tool | **partial** |
| Summarization | 0.85-fraction trigger, keep policy, overflow retry, history preserved to file | 85% high-water compaction, overflow retry, routed model — summary replaces span, history not agent-readable | **partial** |
| Goal re-grounding | — (no equivalent) | Pinned goal block + folded-facts ledger | **ahead** |
| Sub-agents | `task` tool + named `SubAgent` registry, structured response, async parallel variant | `DispatchAgentTool`: isolated child loop, depth/turn/timeout caps, routed model, per-call role/tools args — no persistent named specs | **partial** |
| Skills | SKILL.md dirs on the backend, 3-layer disclosure via `read_file` | SKILL.md dirs, load-on-demand via bespoke `UseSkill` tools, presets in prompt | **partial** |
| Memory | AGENTS.md-style files, self-edited via `edit_file`, store-routed scoping, trust framing | Vector store (SQLite + embeddings) with remember/recall/forget tools + auto-recall budget | **different fork** (see below) |
| Permissions | Declarative first-match glob rules (allow/deny/interrupt), data not code | `RulePolicy` code + `ToolIntent` (richer per-call intent), command allowlists, workspace boundary | **partial** (intent layer is ahead) |
| HITL | approve/**edit**/reject/**respond** over durable interrupts, resumable | Approve/ApproveAlways/Deny over a live channel only | **partial** |
| Sandbox | Backend implementing `execute()`, all fs ops derived; secrets-out posture | Docker command sandbox with limits + refusal-on-degraded (deepagents has no equivalent) | **partial** (posture is ahead) |
| Interpreter / PTC | QuickJS `eval` + programmatic tool calling | Nothing | **absent** |
| Prompt caching | Cache middleware at stack tail, marker-preserving assembly | No explicit cache-aware assembly contract | **absent** |
| Tool-call repair | `PatchToolCallsMiddleware` (automated) | No automated repair, but one built-in re-ask retry: per malformed call (native) or per turn (prompted), then terminal | **partial** |
| Guardrail limits | Model-call / tool-call limit, retry, fallback, PII middleware | Stuck-detection (nudge at 3rd repeat, abort at 5th), max_turns, retry with backoff | **partial** |
| Streaming | Typed projections incl. product-level `subagents` view | `EventSink` wire format, `parent_id`-tagged child events | **partial** |
| Durable execution | Checkpointing: mid-run recovery, indefinite HITL pause, time travel | Session traces (JSONL), no checkpoint/resume | **absent** |
| Per-model profiles | `HarnessProfile`: excluded tools, extra middleware, prompt suffix | Per-backend config knobs (claude-cli effort/session-reuse etc.), no profile abstraction | **partial** |
| MCP | Via LangChain tool integration | First-class `agent-mcp` client crate | **match** |
| Multimodal files | `read_file` returns images/video/audio/documents as content blocks | Text-only end-to-end (read_to_string; String content in ToolOutput/Message); assessed 2026-07-08 (Phase-2 spec J3) | **absent** (deliberately deferred) |

## The three structural gaps

Everything in the table reduces to three missing structures [1][2][3]:

1. **No middleware abstraction** — most "absent" rows (planning, caching,
   tool-call repair, guardrail limits) are one middleware each once a stack
   exists ([middleware composition](/practices/middleware-composition.md)).
2. **No backend/virtual-filesystem seam** — offloading, skills, memory
   files, and sandbox unification all want to be path-routed backends
   ([filesystem as context substrate](/practices/filesystem-as-context-substrate.md)).
3. **No durable state** — resumable HITL, checkpointing, and time travel all
   hang off persistent loop state, which session-trace JSONL does not
   provide [3][1].

## Where the current design should win

The refactor must not regress: the goal/facts-ledger re-grounding, the
`ToolIntent` policy layer (declared access + paths + parsed command per
call — strictly richer than fs-tools-only permissions), refusal-on-degraded
sandbox posture, first-class MCP, and the calibrated token estimator are all
ahead of the deepagents baseline [3][1].

# Citations

1. [deepagents documentation (docs.langchain.com)](/sources/deepagents-docs.md)
2. [deepagents source (langchain-ai/deepagents)](/sources/deepagents-source.md)
3. [rusty-agent — current Rust agent runtime](/sources/rusty-agent-runtime.md)
