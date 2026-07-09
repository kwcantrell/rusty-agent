---
type: Source
title: "Command Scopes | Tauri"
description: "Granular command scope mechanism for controlling permitted and restricted behaviors of Tauri commands."
resource: https://v2.tauri.app/security/scope/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Command scopes represent a granular mechanism for controlling the permitted and restricted behaviors of Tauri commands. The system uses a hierarchical structure with "allow" and "deny" designations, where denial rules always take precedence over allowance rules. Scopes are transmitted to commands, with the command implementation responsible for proper handling and enforcement.

## Scope Type Definition

Scopes must be serializable via `serde`. Plugin developers define scope types specific to their implementations, while application-level scoped commands require developers to define and enforce scope types within their own code. This architecture distributes the responsibility for scope definition across the framework and developer implementations.

## Practical Applications

Different Tauri plugins employ scopes for varying purposes:

- **Fs plugin**: Uses scopes to manage directory and file access permissions with glob-compatible path strings
- **HTTP plugin**: Employs scopes to regulate which URLs can be accessed

This pattern allows each subsystem to define scopes appropriate to its security model and use cases.

## Critical Security Consideration

The documentation emphasizes that "Command developers need to ensure that there are no scope bypasses possible." Scope validation implementations require careful auditing to verify correctness and prevent circumvention. This places responsibility on developers to ensure their scope enforcement logic is bulletproof.

## Example Implementation Pattern

The Fs plugin illustrates scope management through individualized permissions (like `"scope-applocaldata-recursive"`) consolidated into logical sets (such as `"deny-default"`). This enables flexible configuration that applies either globally or to specific commands, allowing developers to express both broad and targeted access policies.

## Hierarchical Enforcement

The precedence rule that denial rules always take priority over allowance rules prevents accidental over-grants and ensures that explicit denials cannot be bypassed by broader allowances.
