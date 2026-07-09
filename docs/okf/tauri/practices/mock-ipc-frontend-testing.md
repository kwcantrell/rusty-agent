---
type: Practice
title: Mock-IPC frontend unit testing
description: Unit-test a Tauri frontend against a fake Tauri environment by intercepting invoke calls, mocking window labels, and mocking events with @tauri-apps/api/mocks — no Rust build, driver, or webview required.
tags: [testing, ipc-architecture]
timestamp: 2026-07-09T00:00:00Z
---

# Mock-IPC frontend unit testing

Frontend unit tests should not spin up a real Tauri backend. Instead, stand up a
"fake" Tauri environment that simulates windows and intercepts IPC calls — the
practice Tauri calls mocking [1]. The `@tauri-apps/api/mocks` module ships the
three tools you need: `mockIPC`, `mockWindows`, and `clearMocks` [1][2]. These run
under a plain DOM test runner (the docs use Vitest), so tests execute fast in
Node with no native webview library involved [3][1].

## Intercept invoke with mockIPC

The most common need is to intercept IPC requests — to assert the correct backend
calls are made and to simulate different backend results [1]. Call `mockIPC(cb)`
with a handler that receives `(cmd, args)` and returns whatever the real Rust
command would return; the return value can be a value or a Promise, so you can
even proxy to `fetch` for fixture data [2]. Register only the commands your test
exercises and branch on `cmd`:

```js
import { mockIPC } from "@tauri-apps/api/mocks";
import { invoke } from "@tauri-apps/api/core";

mockIPC((cmd, args) => {
  if (cmd === "add") return (args.a as number) + (args.b as number);
});
await expect(invoke("add", { a: 12, b: 15 })).resolves.toBe(27);
```

To assert *how many times* — or *whether* — a command was invoked, combine
`mockIPC` with your runner's spy: `vi.spyOn(window.__TAURI_INTERNALS__, "invoke")`
lets you keep the simulated return value and still assert `toHaveBeenCalled()`
[1]. For sidecar or shell commands, grab the event-handler ID from the spawn/
execute args and emit the `Stdout`/`Terminated` events the backend would send back
so the promise resolves [1].

## Mock windows before importing window APIs

Window-specific code (a splash screen, a secondary window) needs fake window
labels. `mockWindows(current, ...others)` mocks the *presence* of windows: the
first argument is the label of the window this JS context believes itself to be
in, and the rest are additional windows [1][2]. In a non-Tauri context you must
call `mockWindows` *before* importing the `@tauri-apps/api/window` module —
otherwise the module initializes without the mock [2]. `mockWindows` only mocks
presence; individual window *properties* (width, height) are mocked as ordinary
IPC calls through `mockIPC` [2].

## Mock events (opt-in, since 2.7.0)

Event mocking is partial and opt-in via the `shouldMockEvents: true` option on
`mockIPC`, added in 2.7.0 [1][2]. With it enabled, `listen` and `emit` are wired
together in-process so a component's `emit` reaches its own `listen` handler,
letting you assert `toHaveBeenCalledWith` on the payload without a real backend
[1][2]. Know the gaps before relying on it: `emitTo` and `emit_filter` are not
supported yet [1][2], and enabling the option consumes any events emitted with
the `plugin:event` prefix [2].

## Reset state between tests with clearMocks

Test runners that reuse a single `window` object across tests leak Tauri-specific
properties (for example the `metadata` set by `mockWindows`) from one test into
the next [2]. Call `clearMocks()` in an `afterEach` hook to reset those properties
and the injected mock functions [2]. Skipping this is a real source of
cross-test contamination — a later test can observe windows a previous test
mocked. jsdom also lacks WebCrypto, so tests that reach code paths needing it
must define `window.crypto.getRandomValues` in a `beforeAll` [1].

## When this is enough, and when it is not

Mock-IPC tests verify that your *frontend* calls the right commands with the right
arguments and reacts correctly to backend responses — they never execute Rust, so
they cannot catch a mismatch between the mocked contract and the real command
signature. Pair them with end-to-end tests
([/practices/webdriver-e2e-testing.md](/practices/webdriver-e2e-testing.md)) that
drive the assembled application, and keep the Rust side covered by Tauri's mock
runtime for Rust unit and integration tests, under which native webview libraries
are not executed [3].

# Citations

1. [Mock Tauri APIs](/sources/tauri-tests-mocking.md)
2. [mocks reference](/sources/tauri-js-mocks-reference.md)
3. [Tests](/sources/tauri-tests-overview.md)
