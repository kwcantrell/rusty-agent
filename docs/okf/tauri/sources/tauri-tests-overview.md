---
type: Source
title: "Tests"
description: "Overview of Tauri unit/integration testing under a mock runtime, plus WebDriver E2E"
resource: https://v2.tauri.app/develop/tests/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---
# Summary

Tests
Tauri offers support for both unit and integration testing utilizing a mock runtime. Under the mock runtime, native
webview libraries are not executed.
See more about the mock runtime here
.
Tauri also provides support for end-to-end testing utilizing the WebDriver protocol.
WebdriverIO Tauri testing
supports Windows, Linux, and macOS; the WebDriver protocol can also be driven directly on Windows and Linux, as macOS
provides no desktop WebDriver client.
See more about WebDriver support here
.
We offer
tauri-action
to help run GitHub actions, but any sort of CI/CD runner can be used with Tauri as long as each
platform has the required libraries installed to compile against.
Edit page
Last updated:
Jun 29, 2026
Previous
Mobile Plugin Development
Next
Mock Tauri APIs
Support on Open Collective
Sponsor on GitHub
© 2026 Tauri Contributors. CC-BY / MIT
