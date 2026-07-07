# Lifecycle phases

The phases of an agent SDLC, from scoping through operations.

- [Deployment](/phases/deployment.md) — The phase in which validated agents are shipped to production through staged rollouts, traffic-isolated revisions, eval-gated pipelines, and agent-specific runtime infrastructure.
- [Design and scoping](/phases/design-and-scoping.md) — The phase where you decide what to build before building it — choosing between workflows and agents, fixing precise agent identity, and selecting an architecture while bounding scope against open-loop failure.
- [Evaluation](/phases/evaluation.md) — The systematic measurement discipline at the center of the agent SDLC, turning subjective judgement into repeatable signal that gates every change and unlocks fast model upgrades.
- [Monitoring and operations (AgentOps)](/phases/monitoring-and-operations.md) — The post-deployment phase where production traces, online evaluation, drift detection, and autonomy measurement keep live agents observable and safe, feeding failures back into evaluation and increasingly using agents themselves as operators.
- [Prototyping and development](/phases/prototyping-and-development.md) — The build phase, where agents grow incrementally from direct API calls into structured systems by developing tools, context, and prompts in tandem under versioning discipline.
- [Testing and safety](/phases/testing-and-safety.md) — The pre-deployment assurance phase that layers sandboxed execution, deterministic and model-based guardrails, permissioning, policy-adherence testing, and multi-tenant isolation to make agent behavior safe before autonomous release.
