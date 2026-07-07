---
type: Practice
title: Spec-first agent workflows
description: Writing an explicit specification before implementation so a probabilistic coding agent solves the right problem and knows when it is done.
tags: [agent-run-sdlc]
timestamp: 2026-07-06T00:00:00Z
---

# Spec-first agent workflows

Spec-first agent workflows separate exploration and planning from implementation. Letting an
agent jump straight to coding can produce code that solves the wrong problem, so a distinct
planning phase should precede execution [1]. This ordering is not a stylistic preference:
frameless, unstructured development of LLM agents introduces critical risks including scope
creep and open-loop failures [3]. Getting intent onto paper first is what prevents the agent
from confidently building the wrong thing.

## Author the spec, then reset context

Once the specification is complete, the implementation should begin in a fresh session. A new
session carries clean context focused entirely on implementation while still referencing the
written spec [1]. This exploits a structural property of the harness: agent performance
degrades as the context window fills, so the pollution accumulated during open-ended
exploration is best discarded before the concrete work begins [1]. The spec becomes the
handoff artifact — a clean slate for the next agent carries a cost only if the artifact holds
enough state for that agent to pick up the work cleanly [2].

## Comprehensive requirements prevent premature completion

A vague goal invites a class of failure specific to autonomous agents: declaring victory
early. Among the primary failure patterns observed in long-running agent development are
declarative victory and premature feature completion [4]. The mitigation is a comprehensive
file of feature requirements written up front; prompting an initializer agent to produce one
prevents agents from prematurely declaring a task complete [4]. The requirement document is
the external standard the agent is measured against — necessary precisely because an agent
tends to praise its own work even when the quality is obviously mediocre to a human observer
[2]. (Independent verification of the finished work is covered in
[/practices/verification-first-agent-coding.md](/practices/verification-first-agent-coding.md).)

## Sprint contracts with explicit completion criteria

At the level of a work chunk, the spec takes the form of a sprint contract: agreeing on what
"done" looks like for that chunk before any code is written [2]. Feature-level decomposition
paired with these explicit contracts improves coherence across a multi-agent system, where a
planner, generator, and evaluator each need a shared definition of the target [2]. Subjective
targets are made gradeable by converting them into concrete criteria — "is this design
beautiful?" is hard to answer consistently, but "does this follow our principles for good
design?" gives the agent something concrete to grade against [2].

## Dynamic specification for probabilistic agents

Because agents are stochastic, a static spec is not enough; runtime specification requires a
dynamic paradigm where high-level intent is concretized at runtime through domain knowledge
injection [3]. Formalisms such as the Agentic Job Description make this explicit, defining the
jurisdictional boundaries, operational contexts, and epistemic evaluation criteria that bound
domain agent execution [3]. Reliability then stems not from the model's internal reasoning
alone but from the engineering structures that anchor stochastic AI within deterministic
business processes [3] — a shift of emphasis from internal model intelligence to the
structured interaction between the agent and its environment [3]. This division of labor
matches how the work actually splits: people decide what to build, and the agent decides how
to build it [5]. The spec is the durable encoding of the "what," and the more domain expertise
its author brings, the more work the agent does per instruction [5].

The spec-first practice is distinct from the runtime meaning of "specification" as a live
contract injected during execution; the two senses are contrasted in
[/comparisons/two-meanings.md](/comparisons/two-meanings.md).

# Citations

1. [Best practices for Claude Code](/sources/anthropic-claude-code-best-practices.md)
2. [Harness design for long-running application development](/sources/anthropic-harness-design-long-running-apps.md)
3. [Agentic Problem Frames: A Systematic Approach to Engineering Reliable Domain Agents](/sources/arxiv-agentic-problem-frames.md)
4. [Effective harnesses for long-running agents](/sources/anthropic-effective-harnesses.md)
5. [Agentic coding and persistent returns to expertise](/sources/anthropic-claude-code-expertise.md)
