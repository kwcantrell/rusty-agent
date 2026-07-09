---
type: Source
title: "Windows Code Signing"
description: "Code signing via OV certificates, Azure Key Vault, and Azure Artifact Signing for Microsoft Store distribution and SmartScreen bypass."
resource: https://v2.tauri.app/distribute/sign/windows/
tags: [distribution]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Code signing on Windows is required to list applications in the Microsoft Store and prevent SmartScreen warnings. Three primary signing methods available:

**OV Certificates** (legacy): "This guide only applies to OV code signing certificates acquired before June 1st 2023." Workflow: convert certificates from `.cer` to `.pfx` format, import into Windows keystore, configure `tauri.conf.json` with certificate thumbprint, digest algorithm, and timestamp URL.

**Azure Key Vault**: Uses relic signing tool. Requires creating a key vault, generating certificate, setting environment variables (`AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET`) for authentication.

**Azure Artifact Signing**: Newest method. Install artifact-signing-cli, set three environment variables (endpoint, account name, certificate profile name). Signing command includes optional description parameter.

## CI/CD Integration

GitHub Actions workflow imports certificates into runner using PowerShell commands: decode base64-encoded certificate files and import into Windows certificate store.

## Configuration

All methods modify `tauri.conf.json` `bundle.windows` section with either certificate details or custom sign command, enabling automated signing during build.

## Technical Requirements

- Certificate conversion from .cer to .pfx format
- Windows certificate store import
- Environment variable configuration for cloud-based signing
- timestamp URL configuration for OV certificates
- Custom signing command support for alternative methods
- GitHub Actions PowerShell certificate import workflow
