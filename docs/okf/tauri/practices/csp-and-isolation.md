---
type: Practice
title: CSP and the isolation pattern
description: Harden the WebView layer of a Tauri app — configure a restrictive Content Security Policy so Tauri can inject hashes and nonces, and reach for the isolation pattern to vet IPC calls from untrusted frontend dependencies.
tags: [security, ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
---

# CSP and the isolation pattern

Capabilities constrain what the frontend *may reach*; CSP and the isolation
pattern harden the frontend itself against injected or malicious code. They are
runtime protections that, together with capabilities, mitigate webview-based
vulnerabilities when handling untrusted content [1]. Use both: a CSP shrinks the
XSS attack surface, and the isolation pattern vets IPC even when a bundled
dependency turns hostile.

## Configure a restrictive CSP

CSP protection activates only when explicitly configured in the Tauri config file
— an unset policy means no protection [2]. The guidance is blunt: "make it as
restricted as possible, only allowing the webview to load assets from hosts you
trust, and preferably own" [2]. Define directives (`default-src`, `connect-src`,
`font-src`, `img-src`, `style-src`, etc.) naming only the hosts you need [2].

Tauri does the cryptographic heavy lifting for you: at compile time it appends its
own nonces and hashes to the relevant CSP attributes for bundled code and assets
[2]. Local scripts are protected by cryptographic hashing (integrity of bundled
code) and external scripts/styles by cryptographic nonces (blocking injected
content even if an external resource is compromised) — a deliberate defense-in-
depth pairing [2]. Two concrete rules follow. First, avoid remote content:
CDN-served scripts and untrusted files "introduce an attack vector," so self-host
critical resources instead of relying on external `script-src` origins [2].
Second, a Rust/WebAssembly frontend must add `'wasm-unsafe-eval'` to `script-src`
to allow WASM instantiation while keeping the rest of the policy intact [2]. The
browser enforces the policy at runtime and reports violations through standard CSP
reporting, which is how you debug a too-tight policy during development [2].

## Reach for the isolation pattern against untrusted dependencies

A CSP does not vet the *IPC calls* your frontend makes — a compromised dependency
can still call legitimate commands maliciously. The isolation pattern closes that
gap: it injects JavaScript running in a sandboxed iframe that intercepts every
frontend-to-Core IPC message, letting a hook function validate or modify it before
it reaches Tauri Core [3]. Its stated purpose is to protect the app "from unwanted
or malicious frontend calls to Tauri Core," and it specifically targets
*development threats* — apps bundle deeply nested dependencies that could execute
malicious code [3]. In the hook you can verify API parameters, restrict file
access to expected paths, validate HTTP headers, and monitor event calls that
trigger Rust [3].

Enable it in config by pointing the isolation pattern at its own dist directory
[3]:

```json
{ "app": { "security": { "pattern": {
  "use": "isolation",
  "options": { "dir": "../dist-isolation" }
} } } }
```

Messages are encrypted with AES-GCM using keys freshly generated at each startup
via SubtleCrypto; overhead is minimal for most apps [3]. Two platform limits to
plan around: on Windows, external files do not load in the sandboxed iframe, so
build-time script inlining is required; and ES Modules do not load — use standard
`<script src>` tags [3]. Fresh keys per startup need adequate system entropy,
which matters for headless test environments [3].

## Keep the isolation app minimal

The isolation application is itself a security boundary, so keep it minimal and
avoid complex dependencies — every dependency you add there widens the supply-
chain attack surface on the very component meant to reduce it [3]. This mirrors the
lifecycle principle that the weakest link defines your security
([/practices/security-threat-model.md](/practices/security-threat-model.md)), and
complements per-window least privilege
([/practices/capabilities-permissions-scopes.md](/practices/capabilities-permissions-scopes.md)).

# Citations

1. [Application Lifecycle Threats](/sources/tauri-security-lifecycle.md)
2. [Content Security Policy (CSP)](/sources/tauri-security-csp.md)
3. [Isolation Pattern](/sources/tauri-ipc-isolation.md)
