---
type: Source
title: "macOS Code Signing"
description: "Apple Developer setup, certificate creation, and CI/CD pipeline configuration for macOS application signing."
resource: https://v2.tauri.app/distribute/sign/macos/
tags: [distribution]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Code signing on macOS enables App Store distribution and prevents browser-download security warnings. Requires an Apple Developer account (paid $99/year or free for development). Three-step certificate workflow: generate CSR on Mac, create certificate in Apple Developer portal (choose "Apple Distribution" for App Store or "Developer ID Application" for outside App Store), download .cer file and install to keychain.

Retrieve signing identity locally:
```
security find-identity -v -p codesigning
```

Configure identity in `tauri.conf.json` under `bundle > macOS > signingIdentity` or via `APPLE_SIGNING_IDENTITY` environment variable.

For CI/CD, export certificate as base64:
```
openssl base64 -A -in /path/to/certificate.p12 -out certificate-base64.txt
```

Set environment variables: `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`. Configure keychain access within CI platform. Notarization requires credentials via App Store Connect API or Apple ID (`APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`).

Ad-hoc signing alternative: configure pseudo-identity `"-"` in signingIdentity for development, but note that "Ad-hoc code signing does not prevent MacOS from requiring users to whitelist the installation in their Privacy & Security settings."

## Key Technical Details

- Certificate signing request (CSR) generated locally on Mac
- "Apple Developer account which is either paid (99$ per year) or on the free plan (only for testing and development purposes)"
- Two certificate types: Apple Distribution (App Store) or Developer ID Application (outside App Store)
- Base64 certificate export required for CI/CD environments
- Notarization mandatory for app distribution outside App Store
- Ad-hoc signing allows development without signing infrastructure but triggers macOS security warnings
