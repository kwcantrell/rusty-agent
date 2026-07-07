# Practices

Named, source-backed practices that recur across orgs.

- [Context engineering](/practices/context-engineering.md) — Treat the context window as a finite, degrading resource and curate the smallest set of high-signal tokens an agent needs at each step.
- [Eval-driven development](/practices/eval-driven-development.md) — Building agents by defining success criteria as evaluations first, then gating every change against a versioned dataset of real and adversarial cases.
- [Harness engineering for long-running agents](/practices/harness-engineering.md) — Building the scaffolding — initialization stages, progress artifacts, context resets, checkpoints, version control, and locks — that carries autonomous agents across context-window boundaries.
- [Human-in-the-loop gates](/practices/human-in-the-loop-gates.md) — Structured human oversight interposes tiered approval gates and checkpoints before risky agent operations, where trustworthy visibility plus simple intervention outperforms mandated interaction formats.
- [LLM-as-judge with human calibration](/practices/llm-as-judge-with-calibration.md) — Grading agent outputs with a rubric-driven model judge that is calibrated to and periodically validated against human judgment, combined with deterministic checks and human review.
- [Memory and state management](/practices/memory-and-state.md) — How agents persist, scope, categorize, and prune memory and state so runs are resumable, personalized, and isolated across tenants.
- [Multi-agent decomposition](/practices/multi-agent-decomposition.md) — Splitting an agentic workload across specialized, coordinated agents instead of one monolith, trading extra tokens for higher quality on complex tasks.
- [Spec-first agent workflows](/practices/spec-first-agent-workflows.md) — Writing an explicit specification before implementation so a probabilistic coding agent solves the right problem and knows when it is done.
- [Start simple, add complexity only when measured](/practices/start-simple.md) — Begin with direct API calls and simple composable patterns, adding orchestration complexity only when evaluation demonstrably shows it improves outcomes.
- [Tool design as an engineering discipline](/practices/tool-design-as-engineering.md) — Tool names, descriptions, and argument shapes steer agent behavior as strongly as prompts, and warrant equal engineering rigor—error-proofing, eval-driven iteration, and minimally viable non-overlapping toolsets.
- [Trajectory evaluation](/practices/trajectory-evaluation.md) — Evaluating an agent's full decision path—tool selection, reasoning, and recovery—against reference trajectories rather than only its final output.
- [Verification-first agent coding](/practices/verification-first-agent-coding.md) — Structuring agent development around machine-checkable pass/fail signals so an autonomous loop can grade its own work and converge on correct behavior.
