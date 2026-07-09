---
type: Source
title: "GitHub Actions Pipeline for Tauri"
description: "CI/CD setup using tauri-action for multi-platform building, signing, and automated release workflows."
resource: https://v2.tauri.app/distribute/pipelines/github/
tags: [distribution]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Continuous integration/deployment for Tauri applications using GitHub Actions. Primary tool is `tauri-action`, which automates building and releasing across platforms. "To set up `tauri-action` you must first set up a GitHub repository." The action can auto-initialize Tauri configuration on repositories without it.

## Core Setup Steps

1. Repository checkout using standard GitHub actions
2. System dependencies for Linux builds: libwebkit2gtk-4.1-dev, libappindicator3-dev, librsvg2-dev, patchelf, xdg-utils
3. Node.js LTS installation with dependency caching
4. Rust toolchain setup with build artifact caching
5. Frontend dependency installation and optional build execution

## Workflow Triggers

Example triggers on `release` branch push. Alternative: git tags matching patterns like `app-v*`. "For a full list of possible trigger configurations, check out the official GitHub documentation."

## Multi-Platform Matrix

Builds for:
- macOS (ARM/Apple Silicon and x64 Intel)
- Ubuntu Linux x64 and ARM64
- Windows x64

Platform-specific compilation arguments defined through matrix strategy.

## Code Signing

"To set up code signing for Windows and macOS in your workflow, follow the specific guide for each platform." Ad-hoc signing recommended for macOS "to avoid macOS treating Apple Silicon builds downloaded from GitHub releases as damaged."

## GitHub Token Permissions

Workflow requires write permissions to create releases. "If this happens, you may need to add write permissions to this token." Configure in project settings under Actions > Workflow permissions.

## ARM Architecture Support

GitHub provides `ubuntu-22.04-arm` runners for public repositories enabling native ARM64 compilation. Private repositories use `pguyot/arm-runner-action` emulation (significantly slower).

## Technical Configuration

- Matrix strategy for platform-specific builds
- Workflow file placement in `.github/workflows/`
- System dependency installation before builds
- Artifact caching for Node.js and Rust
- Platform-specific build arguments
- Automated release creation
