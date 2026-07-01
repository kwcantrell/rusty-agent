# build.md — build/refactor advisor (advisory only)

**This playbook advises; it does not edit.** It supplies patterns, marks where
human judgment must stay in the loop, then hands off to the normal
`superpowers:writing-plans` → `superpowers:test-driven-development` flow.

When building/refactoring a harness component, reach for the matching pattern:

| Building… | Pattern (source in `SKILL.md`) |
|---|---|
| sub-agent orchestration | orchestrator-workers; sub-agents-as-tools; one-feature-at-a-time |
| long-horizon execution | externalized progress artifact over a growing window/compaction |
| tool surface | fewer, consolidated, token-efficient tools; not thin endpoint wrappers |
| context loadout | static vs dynamic split; progressive-disclosure skills |
| workflow vs agent choice | predefined code paths (workflow) vs self-directed (agent) |

**The 80% problem:** AI gets ~80%; the remaining 20% — edge cases, error
handling, integration, subtle correctness — needs human judgment. Name, in the
plan, exactly where that 20% lives for this component and keep a human on it.

**Handoff:** stop here and invoke `superpowers:writing-plans` to turn the chosen
patterns into a step-by-step plan, then implement under
`superpowers:test-driven-development`. This playbook does not drive edits itself.
