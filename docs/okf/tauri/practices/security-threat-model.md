---
type: Practice
title: Threat modeling a Tauri app
description: Treat Tauri's smaller attack surface as a starting point, not a guarantee — model lifecycle threats, scope the asset protocol, stay on patched versions against origin-confusion, and learn from the Bishop Fox XSS-to-RCE chain.
tags: [security]
timestamp: 2026-07-09T00:00:00Z
---

# Threat modeling a Tauri app

Tauri is positioned as a lighter, security-first alternative to Electron, but the
attack surface does not disappear [1]. The framework's whitelist model — APIs off
by default, explicitly enabled — is real protection, yet that security "depends on
developer understanding and correct configuration," and misconfigured apps remain
exploitable [1]. Threat-model your app as though the frontend can be compromised
and the version can be behind, because both have produced real exploits.

## Model the whole lifecycle, not just runtime

"The weakest link in your application lifecycle essentially defines your security"
[2]. Threats span upstream dependencies, the development environment, the build,
and distribution — not only runtime [2]. Concretely: keep dependencies updated and
evaluate third-party libraries, harden dev environments, aim for reproducible
builds, protect source repositories, and only trust CI/CD systems you have vetted
[2]. Runtime protections (CSP and capabilities) are the last layer, mitigating
webview vulnerabilities when handling untrusted content — not the whole story [2].
Tauri's own trust model reinforces where to focus: the Rust core is unconstrained,
the WebView is restricted, and every byte crossing the IPC boundary is inspected
and strongly typed to prevent trust-boundary violations [3].

## Learn from the XSS-to-RCE chain

Bishop Fox demonstrated that a misconfigured Tauri app is fully exploitable
despite the framework's safeguards [1]. The chain combined three ingredients:
filesystem write capabilities, the `shell.open()` function, and permissive
directory scope permissions [1]. An SVG-based XSS in a markdown editor gave
arbitrary JS execution in the app's security context; from there the attacker
extracted the username via error messages, wrote a reverse-shell payload to disk,
and executed it through the system's default handler — user-level RCE [1]. The
lesson is directly actionable: the presence of `shell.open` plus a loose fs scope
turns any XSS into RCE, so scope filesystem and shell permissions tightly
([/practices/capabilities-permissions-scopes.md](/practices/capabilities-permissions-scopes.md))
and shrink the XSS surface with a strict CSP
([/practices/csp-and-isolation.md](/practices/csp-and-isolation.md)). Bishop Fox's
own recommendations: prioritize configuration-file analysis in assessments,
understand the API attack surface before deployment, test across all supported
platforms, and verify apps run current Tauri versions with up-to-date dependencies
[1].

## Scope the asset protocol deliberately

The asset protocol controls which filesystem paths the WebView can be served, and
it is another place an over-broad scope leaks the disk [4]. Enable it explicitly
and define a scope with glob patterns — either an allow array or an allow/deny
object — using base-directory variables like `$HOME` and `$RESOURCE`, and note the
special handling for Unix dot-prefixed directories [4]. Static `tauri.conf.json`
scope covers fixed paths; when a user *selects* a path at runtime, the
persisted-scope plugin maintains that access across restarts [4].

## Stay patched: origin confusion (CVE-2026-42184)

Version currency is not hygiene theater. CVE-2026-42184 (GHSA-7gmj-67g7-phm9,
CVSS 6.1) let remote pages invoke local-only IPC commands on Windows and Android
[5]. On those platforms Tauri maps custom schemes to `http://<scheme>.localhost/`,
and the vulnerable `is_local_url()` used `split_once('.')` — examining only the
first label. A page at `http://app.evil.com/` was misclassified as `Origin::Local`
if the app registered an `app` scheme, so a remote domain could spoof a local
origin and reach commands meant only for the trusted frontend [5]. Vulnerable
versions are Tauri ≥2.0 through ≤2.11.0; the fix landed in ≥2.11.1 [5]. The
practice is simply to track Tauri security advisories and stay on a patched
release — an old version silently reopens the local/remote trust boundary that
capabilities assume is intact.

# Citations

1. [Beyond Electron: Attacking Alternative Desktop Application Frameworks](/sources/bishopfox-beyond-electron.md)
2. [Application Lifecycle Threats](/sources/tauri-security-lifecycle.md)
3. [Security overview](/sources/tauri-security-overview.md)
4. [Asset protocol scope](/sources/tauri-security-asset-protocol.md)
5. [Origin Confusion Allows Remote Pages to Invoke Local-Only IPC Commands](/sources/ghsa-local-url-origin-confusion.md)
