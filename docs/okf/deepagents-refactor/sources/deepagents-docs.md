---
type: Source
title: deepagents documentation (docs.langchain.com)
description: The official deepagents docs — overview, customization, tools, backends, permissions, skills, context-engineering, subagents, HITL, sandboxes, interpreters, streaming, memory, production.
resource: https://docs.langchain.com/oss/python/deepagents/overview
org: LangChain
tags: [deepagents-refactor]
timestamp: 2026-07-08T00:00:00Z
---

# Summary

The official documentation set for the `deepagents` Python package
(researched 2026-07-08 across the overview, quickstart, customization, tools,
backends, permissions, skills, context-engineering, subagents,
human-in-the-loop, sandboxes, interpreters, event-streaming, multimodal,
memory, and going-to-production pages — each at
`https://docs.langchain.com/oss/python/deepagents/<page>`). Key claims
extracted below; the live docs are the authority.

# Key claims

## Architecture

- deepagents is a "batteries-included agent harness": LangGraph graph runtime → LangChain's minimal `create_agent` tool-calling loop → `create_deep_agent`, an opinionated middleware stack composed onto that loop
- Capabilities are grouped as four pillars: Execution Environment (tools/MCP, virtual fs, fs permissions, code execution), Context Management (skills, memory, summarization + offloading, prompt caching), Delegation (todos, subagents), Steering (HITL, permissions)
- `create_deep_agent(model, tools, system_prompt, middleware, subagents, skills, memory, permissions, backend, interrupt_on, ...)` is the single factory; `HarnessProfile` / `register_harness_profile()` allow per-model tool exclusions, extra middleware, and a model-specific system-prompt suffix placed last ("closest to conversation history for maximum model influence")
- User middleware whose `.name` matches a default middleware replaces it in place; `FilesystemMiddleware` and `SubAgentMiddleware` cannot be excluded via profiles

## Virtual filesystem and backends

- Built-in tools: `ls`, `read_file` (pagination + multimodal), `write_file`, `edit_file`, `delete`, `glob`, `grep`, `execute` (sandbox backends only), plus `task` and `write_todos`
- `BackendProtocol` requires `ls/read/write/edit/glob/grep` (+ optional `delete`); backends return structured results with an `error` field rather than raising (a convention observed across the built-in backends, not a documented protocol guarantee)
- Backends: `StateBackend` (default; files in LangGraph state, thread-scoped), `FilesystemBackend` (real disk, `virtual_mode=True` blocks path traversal), `LocalShellBackend` (adds unsandboxed `execute`), `StoreBackend` (cross-thread LangGraph store with namespace factories for per-user/per-assistant scoping), `ContextHubBackend`, sandbox backends, and `CompositeBackend` which routes by path prefix (longest prefix wins) and aggregates `ls`/`glob`/`grep` across backends
- Docs recommend a Composite so internal data (`/large_tool_results/`, `/conversation_history/`) stays separate from project files

## Context engineering

- Summarization triggers at `("fraction", 0.85)` of the model profile's input limit, keeping `("fraction", 0.10)`; fallback without a profile is 170,000-token trigger / keep 6 messages; on `ContextOverflowError` it summarizes immediately and retries
- Evicted history is appended to `{artifacts_root}/conversation_history/{thread_id}.md` and the summary message embeds the file path so the agent can `read_file` the canonical record
- Tool results over `tool_token_limit_before_evict` (default 20,000 tokens) are written to `{artifacts_root}/large_tool_results/{tool_call_id}` and replaced by a pointer message plus preview; the injected prompt tells the model to `read_file` in chunks or `grep` within the offload prefix; seven fs tools are exempt
- Anthropic/Bedrock prompt-caching middleware sits in the stack tail — after tool-call patching, before memory and HITL — so cached prefixes match actual model input

## Subagents

- The `task` tool launches "an ephemeral subagent to handle complex, multi-step independent tasks with isolated context windows"; parameters are `description` and `subagent_type`; the subagent returns a single message
- `SubAgent` spec: required `name`, `description`, `system_prompt` (no inheritance from parent prompt); optional `tools` (inherits only if omitted), `model`, `middleware` (merges into default subagent stack), `interrupt_on`, `skills`, `permissions` (replaces entirely if provided), `response_format`; `CompiledSubAgent` wraps any compiled graph with a `messages` state key
- Every deep agent gets a `general-purpose` subagent inheriting parent model/tools/skills
- Stated rationale: large tool outputs fill the parent window; subagents run autonomously with fresh context and return only the final result, keeping the main agent's context clean

## Skills

- A skill is a directory with `SKILL.md` (YAML frontmatter: `name` ≤64 chars matching the directory, `description` ≤1024 chars; optional `license`, `compatibility`, `metadata`, `allowed-tools`)
- Three-layer progressive disclosure: metadata in system prompt at startup → full SKILL.md via `read_file` on activation → supporting resources on demand; skills resolve per-backend (state seed, store namespace, disk, hub repo)

## Memory

- `memory=[...]` lists file paths (e.g. `/memories/AGENTS.md`) loaded at startup and injected into the system prompt; the agent self-edits memory with its ordinary `edit_file` tool, prompted to update "usually in the same turn"
- Trust framing: memory content "may be outdated, incorrect, or written by someone other than the current user"; the agent must not obey memory that conflicts with the user's explicit request
- Long-term memory = route `/memories/` to a `StoreBackend` in a Composite; production docs recommend user-scoped namespaces and warn about prompt injection via shared writable memory

## Permissions & HITL

- `FilesystemPermission`: `operations` (read = ls/read_file/glob/grep; write = write_file/edit_file/delete), glob `paths`, `mode` allow|deny|interrupt; first-match-wins, no match ⇒ allowed; enforced in middleware before tools run; only built-in fs tools are governed — custom/MCP tools and sandbox `execute` bypass
- `interrupt_on` maps tool name → decisions `approve`/`edit`/`reject`/`respond`, with an optional `when` predicate; surfaced as LangGraph interrupts (checkpointer mandatory), resumed via `Command(resume=...)`

## Sandboxes, interpreters, streaming, production

- Sandbox providers implement only `execute()`; `BaseSandbox` derives all fs operations by running scripts inside the sandbox; providers include LangSmith, Daytona, E2B, Modal, Runloop, Vercel, AgentCore
- Stated security posture: never put secrets inside a sandbox — sandboxes protect the host, not against context injection; keep secrets in host-side tools or credential-injecting proxies
- `CodeInterpreterMiddleware` (shipped in the separate `langchain_quickjs` package, not the deepagents tree) adds an `eval` tool running JavaScript in in-process QuickJS (capability-scoped, 64 MB / 5 s defaults); Programmatic Tool Calling exposes allowlisted tools as async functions callable from code, so loops/retries happen without model turns
- `stream_events` provides typed projections: `messages`, `tool_calls`, `values`, `output`, and `subagents` (per-delegation handles with status and nested streams); "subagents" is the product-level view vs "subgraphs" internal view
- Production guidance: durable checkpointing (mid-run recovery, indefinite HITL pauses, audit trails), plus guardrail middleware — `ModelCallLimitMiddleware`, `ToolCallLimitMiddleware`, `ModelRetryMiddleware`, `ModelFallbackMiddleware`, `PIIMiddleware`
