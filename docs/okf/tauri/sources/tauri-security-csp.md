---
type: Source
title: "Content Security Policy (CSP) | Tauri"
description: "Content Security Policy implementation in Tauri to mitigate cross-site-scripting and related web vulnerabilities."
resource: https://v2.tauri.app/security/csp/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri implements Content Security Policy restrictions on HTML pages to mitigate common web vulnerabilities, particularly cross-site-scripting (XSS) attacks. The framework employs two primary security strategies: local scripts are protected through cryptographic hashing, and external scripts and styles utilize cryptographic nonces to prevent unauthorized content loading.

## Protection Mechanisms

CSP protection activates only when explicitly configured in the Tauri configuration file. Developers should "make it as restricted as possible, only allowing the webview to load assets from hosts you trust, and preferably own."

The dual-mechanism approach provides defense-in-depth:

- **Cryptographic hashing** for local scripts ensures integrity of bundled code
- **Cryptographic nonces** for external resources prevent script injection even if external resources are compromised

## Automatic Processing

"At compile time, Tauri appends its nonces and hashes to the relevant CSP attributes automatically to bundled code and assets," reducing developer burden. Developers define the policy directives, and the framework automatically injects the necessary cryptographic material at build time.

## Security Warnings

Loading remote content like CDN-served scripts "introduce an attack vector" and untrusted files can create unpredictable vulnerabilities. The documentation cautions against architectural decisions that rely on external script sources, emphasizing self-hosting of critical resources.

## WebAssembly Consideration

Applications using Rust-based frontends or WebAssembly should include `'wasm-unsafe-eval'` in their `script-src` directive to enable WebAssembly instantiation while maintaining other CSP protections.

## Configuration Example

The documentation provides a sample configuration from Tauri's API example showing directives for:

- `default-src`: Default fallback source restrictions
- `connect-src`: Allowed connection endpoints
- `font-src`: Font loading sources
- `img-src`: Image source restrictions
- `style-src`: Stylesheet loading sources

Each directive specifies the hosts from which resources can be loaded, enabling fine-grained control over the webview's access to external resources.

## CSP Enforcement

CSP policies are enforced by the browser engine at runtime. Violations of the configured policy are reported through the browser's standard CSP reporting mechanisms, allowing developers to detect and debug policy violations during development and deployment.
