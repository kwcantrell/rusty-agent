---
type: Source
title: "I Built a WebDriver for WKWebView Tauri Apps on macOS"
description: "Open-source W3C WebDriver implementation for end-to-end testing of Tauri applications on macOS."
resource: https://danielraffel.me/2026/02/14/i-built-a-webdriver-for-wkwebview-tauri-apps-on-macos/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Daniel Raffel describes creating Tauri-WebDriver, an open-source W3C WebDriver implementation enabling end-to-end testing for Tauri applications on macOS. The project addresses a gap in Apple's testing tooling for desktop apps using embedded WKWebView components. Raffel notes: "Apple's WebDriver story is safaridriver for automating Safari itself, which doesn't help when your UI is a WKWebView inside a desktop app."

## The Problem

While web developers have established testing frameworks like Selenium and WebDriverIO for browser automation, macOS lacked native WebDriver support for WKWebView-based Tauri applications. This gap left Tauri developers without standard end-to-end testing capabilities on Apple's platform.

## The Solution: Tauri-WebDriver

Tauri-WebDriver comprises two Rust components working in tandem:

### 1. Tauri Plugin (In-App Component)
- Runs inside debug builds of Tauri applications
- Starts an HTTP server listening for WebDriver protocol commands
- Injects JavaScript bridge enabling DOM interaction and inspection

### 2. CLI Binary (`tauri-wd`)
- Implements the W3C WebDriver protocol specification
- Listens on port 4444 for automation requests
- Translates WebDriver commands to plugin HTTP requests
- Executes within the application context

## Architecture Pattern

The two-component design separates concerns:
- **Plugin**: Provides in-app instrumentation and DOM access
- **CLI**: Exposes standard WebDriver API to test clients

This mirrors the Selenium/WebDriver architecture while tailored to Tauri's plugin ecosystem.

## AI Integration

The project includes MCP (Model Context Protocol) integration via `mcp-tauri-automation`, enabling AI agents like Claude Code to directly interact with Tauri applications for automated testing workflows. This extends testing automation to LLM-driven agent scenarios.

## Ecosystem Context

Raffel acknowledges competing solutions emerged simultaneously with his project:

1. **CrabNebula's Commercial Offering**: Cross-platform solution requiring subscription
2. **Alternative Open-Source Plugin**: Supporting cross-platform compatibility

Despite the timing overlap, Raffel emphasizes the learning value of the implementation, stating the project deepened his understanding of:
- Rust ecosystem and language patterns
- Tauri's plugin architecture
- WKWebView behavior and constraints
- W3C WebDriver protocol implementation

## Technology Stack

- **Language**: Rust (both plugin and CLI)
- **Protocol**: W3C WebDriver specification
- **Scope**: macOS WKWebView automation
- **Integration**: MCP for AI agent automation

## Significance

The project represents a DIY approach to bridging macOS WebDriver tooling gaps in the Tauri ecosystem, demonstrating how protocol implementation and plugin architecture can enable end-to-end testing where platform-native tooling falls short.
