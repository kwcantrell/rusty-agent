---
type: Source
title: "Continuous Integration | Tauri"
description: "GitHub Actions workflow for executing WebDriver tests with tauri-driver on Linux and Windows CI systems."
resource: https://v2.tauri.app/develop/tests/webdriver/ci/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

This guide demonstrates executing WebDriver tests using `tauri-driver` within continuous integration systems, with a focus on GitHub Actions workflow implementation. The documentation establishes that "It is possible to run WebDriver tests with `tauri-driver` on your CI," utilizing the WebdriverIO example from prior documentation.

## Prerequisites

- Tauri application located in the `src-tauri` folder
- WebDriverIO test runner in the `e2e-tests` directory, invoked via `yarn test`

## GitHub Actions Workflow Configuration

The provided workflow implements a multi-platform testing strategy with `fail-fast: false` to allow all matrix runs to complete independently despite individual failures.

### System Setup Steps

**For Linux:**
- Checkout code
- Install Linux dependencies: `webkit2gtk-driver`, `xvfb`
- Configure Rust toolchain with caching

**For Windows:**
- Checkout code
- Install Microsoft Edge Driver using `msedgedriver-tool`
- Configure Rust toolchain with caching

### Development Environment

- Install Node.js (version 24 in the example)
- Install project dependencies via Yarn
- Install `tauri-driver`

### Test Execution

**Linux:** Tests run through `xvfb-run` for headless display server functionality. The documentation notes: "The WebDriver tests are executed on Linux by creating a fake display."

**Windows:** Tests execute directly without additional display configuration.

```yaml
# Example workflow structure
name: WebDriver Tests
on: [push, pull_request]
jobs:
  test:
    runs-on: [ubuntu-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - name: Install Linux dependencies
        if: runner.os == 'Linux'
        run: sudo apt-get install webkit2gtk-driver xvfb
      - name: Install Edge Driver
        if: runner.os == 'Windows'
        run: npm install -g msedgedriver-tool
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      - name: Install Node
        uses: actions/setup-node@v4
        with:
          node-version: '24'
      - name: Install dependencies
        run: yarn install
      - name: Install tauri-driver
        run: npm install -g tauri-driver
      - name: Run tests
        run: xvfb-run -a yarn test
```

## Key Technical Details

- Uses `xvfb` (X Virtual Framebuffer) on Linux for headless display emulation
- Separate matrix jobs for platform-specific driver installation
- Caching of Rust and Node dependencies for faster CI runs
- Coordination of Tauri application build and test execution phases
