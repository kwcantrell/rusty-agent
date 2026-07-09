---
type: Source
title: "Distribute | Tauri"
description: "Comprehensive tooling for releasing Tauri applications to app stores or as platform-specific installers."
resource: https://v2.tauri.app/distribute/
tags: [distribution]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

The Distribute section provides comprehensive tooling for releasing applications to app stores or as platform-specific installers. Tauri streamlines the entire distribution workflow from building through signing to final deployment.

## Building & Bundling

The CLI handles application compilation via:
- `npm run tauri build`
- `yarn tauri build`
- `pnpm tauri build`
- `cargo tauri build`

For customized bundling, developers can separate build and bundle steps:

```bash
npm run tauri build -- --no-bundle
npm run tauri bundle -- --bundles app,dmg
```

## Versioning

Versions are managed through `tauri.conf.json > version` configuration. If unset, Tauri defaults to the `src-tauri/Cargo.toml` package version. Platform-specific limitations may apply.

## Code Signing

Code signing enhances the security of your application by applying a digital signature and is required on most platforms. Documentation covers:
- macOS (signing and notarization)
- Windows
- Linux
- Android
- iOS

## Platform-Specific Distribution

**Linux**: AppImage, AUR, Debian, RPM, Snapcraft

**macOS**: App Store, DMG installers (both require signing; non-App Store requires notarization)

**Windows**: Microsoft Store, Windows Installer

**Android/iOS**: Google Play and App Store respectively

**Cloud Services**: CrabNebula Cloud for global distribution with auto-updates
