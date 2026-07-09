---
type: Source
title: "Security | Tauri"
description: "Overview of Tauri's security architecture, trust boundaries, access control mechanisms, and vulnerability disclosure process."
resource: https://v2.tauri.app/security/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri implements a multi-layered security model distinguishing between Rust core code and WebView frontend code. The framework enforces that "data passed between boundaries is inspected and strongly defined to prevent trust boundary violations" through an IPC layer serving as the controlled bridge between trust domains.

## Trust Boundaries

Tauri maintains a strict separation between the Rust core, which has full system resource access in an unconstrained execution environment, and the WebView frontend, which is restricted to exposed system resources via IPC. Access is controlled through capability configurations with fine-grained access levels enforced at the command implementation level.

## WebView Architecture

Rather than bundling WebViews, Tauri relies on operating system-provided WebViews. The framework notes that "WebView package maintainers are significantly faster to patch and roll out security updates than application developers who bundle WebView directly," reducing the attack surface and maintenance burden for developers.

## Access Control Mechanisms

Tauri provides multiple layered security controls:

- **Permissions**: Descriptions of explicit privileges for commands
- **Command Scopes**: Granular control over command behavior and resource access
- **Capabilities**: Window and webview-level permission grants
- **Asset Protocol Scope**: Control over asset loading
- **Content Security Policy (CSP)**: Mitigation of web vulnerabilities like XSS
- **HTTP Headers**: Additional security headers for web resources
- **Runtime Authority**: Authority checks at the runtime level

## Vulnerability Disclosure

The team requests coordinated security disclosure through GitHub Vulnerability Disclosure (preferred) or email to security@tauri.app. The documentation emphasizes: "please do not publicly comment on your findings" regarding security concerns, ensuring responsible disclosure practices.
