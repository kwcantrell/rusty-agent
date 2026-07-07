---
type: Source
title: Evaluate your AI agents with Vertex Gen AI evaluation service
description: Google Cloud material on the SDLC of AI agents.
resource: https://cloud.google.com/blog/products/ai-machine-learning/introducing-agent-evaluation-in-vertex-ai-gen-ai-evaluation-service
org: Google Cloud
tags: [building-agents]
timestamp: 2026-07-06T00:00:00Z
---

# Summary

First-party material by Google Cloud (published 2025-01-24). Key claims extracted below; the live document is the authority.

# Key claims

- Agent evaluation must assess both the reasoning and decision-making processes behind actions, not just final outputs
- Agent evaluation divides into two categories: assessment of final responses and assessment of action trajectories
- Custom success criteria are necessary for final response evaluation to match specific business requirements
- Trajectory evaluation provides six distinct metrics for comprehensive action path analysis at varying strictness levels
- Exact-match trajectory metrics require perfect replication of ideal action sequences
- In-order trajectory metrics verify action correctness and sequence without penalizing extra steps
- Any-order trajectory metrics validate action completeness independent of execution order
- Precision and recall metrics measure action accuracy and completeness respectively
- Single-tool use metrics assess whether agents have learned to adopt specific tools or capabilities
- Evaluation datasets must contain four key elements: user prompts, reference trajectories, generated trajectories, and responses
- Standard text generation metrics are insufficient for agent evaluation in interactive environments
- Custom evaluation metrics enable environment-specific effectiveness assessment for agent responses
- Multi-framework support across major agent development ecosystems enables standardized evaluation
- Experiment tracking integrates evaluation results into managed tracking services for SDLC workflow integration
- Detailed metrics dashboards provide granular visibility into agent performance across dimensions
