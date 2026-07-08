# Practices

The deepagents capability patterns the refactor should encode, each grounded
in the docs/source and mapped against the current runtime.

- [Middleware composition over monolithic loops](/practices/middleware-composition.md) — Build the harness as an ordered middleware stack — each unit shipping tools, state, prompt fragments, and call wrappers — composed onto a minimal loop by a single factory.
- [Planning by recitation (the no-op todo tool)](/practices/planning-by-recitation.md) — A write_todos tool that performs no computation; rewriting the plan into recent context keeps long-horizon goals in the attention window.
- [Filesystem as the context substrate](/practices/filesystem-as-context-substrate.md) — One pluggable virtual filesystem behind all file tools; evicted history, oversized results, skills, and memory all become files routed across backends by path prefix.
- [Sub-agent context quarantine](/practices/subagent-context-quarantine.md) — Ephemeral subagents with isolated context and single-message handoff, organized as a named registry with per-agent prompt/tools/model/permissions.
- [Progressive skill disclosure](/practices/progressive-skill-disclosure.md) — Skills as SKILL.md directories on the filesystem, disclosed in three layers: metadata at startup, body on activation, resources on demand.
- [Memory as agent-editable files](/practices/memory-as-editable-files.md) — AGENTS.md-style memory files loaded into the prompt, self-edited via edit_file, scoped by store routing, with explicit trust framing.
- [Declarative guardrails — permissions and interrupt-driven HITL](/practices/declarative-guardrails.md) — Steering as data: first-match-wins permission rules plus per-tool interrupts with approve/edit/reject/respond decisions.
- [Sandboxed execution behind the filesystem seam](/practices/sandboxed-execution.md) — Sandboxes implement only execute() and inherit derived file operations; interpreters add capability-scoped code execution and programmatic tool calling.
- [Threshold summarization with cache-aware assembly](/practices/summarization-and-caching.md) — Fraction-of-window summarization with keep policy and overflow retry, history preserved to readable files, caching-aware prompt ordering.
