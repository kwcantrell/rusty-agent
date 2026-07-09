# Practices

Actionable, source-backed guidance for building, securing, shipping, and testing
Tauri v2 applications.

## Testing

* [Mock-IPC frontend unit testing](/practices/mock-ipc-frontend-testing.md) - Unit-test a Tauri frontend against a fake Tauri environment by intercepting invoke calls, mocking window labels, and mocking events with @tauri-apps/api/mocks.
* [WebDriver end-to-end testing](/practices/webdriver-e2e-testing.md) - Drive an assembled Tauri app over W3C WebDriver — choosing the @wdio/tauri-service or driving tauri-driver directly, and working around the macOS WKWebView gap.
* [Running WebDriver tests in CI](/practices/webdriver-ci.md) - Configure GitHub Actions to run tauri-driver tests across Linux and Windows with webkit2gtk-driver, a synced Edge driver, and an xvfb virtual display.

## Security

* [Capabilities, permissions, and scopes discipline](/practices/capabilities-permissions-scopes.md) - Configure Tauri's access-control system for least privilege — per-window and per-platform permissions, path- and host-scoped commands, and pruned defaults.
* [CSP and the isolation pattern](/practices/csp-and-isolation.md) - Harden the WebView with a restrictive Content Security Policy and reach for the isolation pattern to vet IPC calls from untrusted frontend dependencies.
* [Threat modeling a Tauri app](/practices/security-threat-model.md) - Treat the smaller attack surface as a starting point — model lifecycle threats, scope the asset protocol, stay patched against origin-confusion, and learn from the Bishop Fox XSS-to-RCE chain.

## IPC & architecture

* [Command design and events-vs-channels](/practices/command-design.md) - Design the Rust/JS boundary — type-safe commands with Result-based error handling, and choosing events vs channels by data volume within the JSON-serialization ceiling.
* [State management and the Channel::send gotcha](/practices/state-management.md) - Share mutable state via the Manager API — preferring std Mutex, avoiding Arc, dodging the runtime-panic type mismatch, and knowing Channel::send can block.

## Distribution & updates

* [Code signing for macOS and Windows](/practices/code-signing.md) - Sign and notarize so users are not blocked by OS security warnings — the macOS certificate/notarization workflow, Windows OV/Azure options, and CI wiring via base64 certificates.
* [Release pipeline and auto-updates](/practices/release-pipeline-and-updates.md) - Ship end to end — a multi-platform GitHub Actions build via tauri-action, and the updater plugin with its mandatory signatures, endpoint variables, and install modes.

## Performance & footprint

* [Binary size and footprint](/practices/binary-size-and-footprint.md) - Shrink the binary with Cargo release-profile levers and unused-command removal, and read the Tauri-vs-Electron benchmarks skeptically since WebKit memory can exceed Chromium's.
