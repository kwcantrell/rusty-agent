---
type: Source
title: "Tauri Updater Plugin"
description: "Automatic application updates via dynamic server or static JSON with mandatory cryptographic signature validation."
resource: https://v2.tauri.app/plugin/updater/
tags: [distribution]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

The Tauri updater plugin enables: "Automatically update your Tauri app with an update server or a static JSON."

Platform support: Windows, Linux, macOS (full support); Android, iOS (lower support). Minimum Rust 1.77.2 required.

## Security Model

Signature validation is mandatory and cannot be disabled. Uses public-private key pair:
- Public key validates artifacts before installation
- Private key signs installer files (must remain confidential)
- Key loss prevents publishing future updates to existing installations

## Configuration Requirements

Three critical elements:

1. **Public Key**: "This has to be the public key generated from the Tauri CLI in the step above. It **cannot** be a file path!"
2. **Endpoints**: URLs supporting dynamic variables (`{{current_version}}`, `{{target}}`, `{{arch}}`)
3. **createUpdaterArtifacts**: Boolean flag enabling update bundle generation during builds

## Server Response Formats

**Static JSON** requires: version (SemVer format), platforms object with OS-ARCH keys, signature content, and update URL.

**Dynamic servers** respond with HTTP 204 (no update available) or 200 with JSON containing version, URL, and signature. "Your server should respond with a status code of `204 No Content` if there is no update available."

## Windows Installation Modes

Three configurable modes: passive (default with progress bar), basicUi (requires interaction), quiet (no feedback, limited privilege escalation).

## Critical Implementation Notes

- Environment variables (not .env files) provide private key during builds
- Windows automatically exits applications before installation
- Custom targets can override default OS-ARCH platform matching
- Version comparison logic is customizable (permits downgrades)
- Public key embedded in binary at compile time
