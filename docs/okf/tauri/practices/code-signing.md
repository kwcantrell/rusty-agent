---
type: Practice
title: Code signing for macOS and Windows
description: Sign and notarize Tauri apps so users are not blocked by OS security warnings — the macOS certificate-and-notarization workflow, the Windows OV/Azure signing options, and how to wire both into CI via base64-exported certificates.
tags: [distribution, security]
timestamp: 2026-07-09T00:00:00Z
---

# Code signing for macOS and Windows

Code signing applies a digital signature to your app and is required on most
platforms [1]. Skipping it does not just look unprofessional — it actively blocks
users behind OS security warnings (macOS Gatekeeper, Windows SmartScreen) and bars
you from the app stores [2][3]. Sign locally to validate the setup, then move the
credentials into CI so every release is signed automatically.

## macOS: sign, then notarize

macOS signing requires an Apple Developer account — paid ($99/year) for
distribution, or the free plan for testing/development only [2]. The certificate
workflow is three steps: generate a Certificate Signing Request on the Mac, create
the certificate in the Apple Developer portal, and download/install the `.cer` into
your keychain [2]. Pick the certificate *type* by target: **Apple Distribution**
for the App Store, **Developer ID Application** for distribution outside it [2].
Find your installed identity with `security find-identity -v -p codesigning` and
set it in `tauri.conf.json` under `bundle > macOS > signingIdentity`, or via the
`APPLE_SIGNING_IDENTITY` env var [2].

Signing alone is not enough outside the App Store: **notarization is mandatory for
distribution outside the App Store** [2]. Supply notarization credentials via the
App Store Connect API or an Apple ID (`APPLE_ID`, `APPLE_PASSWORD`,
`APPLE_TEAM_ID`) [2]. For development without signing infrastructure, ad-hoc
signing (`signingIdentity: "-"`) works, but it "does not prevent macOS from
requiring users to whitelist the installation in their Privacy & Security
settings" — so it is a dev convenience, not a distribution strategy [2].

## Windows: OV, Azure Key Vault, or Azure Artifact Signing

Windows signing is required to list in the Microsoft Store and to avoid SmartScreen
warnings [3]. Three methods, differing mainly in where the key lives [3]:

- **OV certificates** — legacy, and the guide applies only to OV certs acquired
  before June 1st 2023. Convert `.cer` to `.pfx`, import into the Windows keystore,
  and configure `tauri.conf.json` with the certificate thumbprint, digest
  algorithm, and a timestamp URL [3].
- **Azure Key Vault** — uses the relic signing tool; create a vault, generate a
  certificate, and set `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET`
  for auth [3].
- **Azure Artifact Signing** — the newest method; install artifact-signing-cli and
  set the endpoint, account name, and certificate profile name [3].

All methods write to the `bundle.windows` section of `tauri.conf.json`, either as
certificate details or a custom sign command, so signing runs automatically during
`tauri build` [3].

## Wire signing into CI

The portable pattern for both platforms is to export the certificate as base64 and
inject it as a secret. On macOS: `openssl base64 -A -in certificate.p12 -out
certificate-base64.txt`, then set `APPLE_CERTIFICATE` and
`APPLE_CERTIFICATE_PASSWORD` and configure keychain access in the CI platform [2].
On Windows, a GitHub Actions step decodes the base64 certificate with PowerShell
and imports it into the runner's certificate store [3]. This keeps private keys out
of the repo while letting the release pipeline
([/practices/release-pipeline-and-updates.md](/practices/release-pipeline-and-updates.md))
produce signed artifacts on every build.

# Citations

1. [Distribute overview](/sources/tauri-distribute-overview.md)
2. [macOS Code Signing](/sources/tauri-sign-macos.md)
3. [Windows Code Signing](/sources/tauri-sign-windows.md)
