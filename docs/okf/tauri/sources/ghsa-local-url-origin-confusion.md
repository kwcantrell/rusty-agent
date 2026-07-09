---
type: Source
title: "Origin Confusion Allows Remote Pages to Invoke Local-Only IPC Commands"
description: "CVE-2026-42184: Tauri is_local_url() validation flaw on Windows/Android allows remote domains to spoof local origins."
resource: "https://github.com/tauri-apps/tauri/security/advisories/GHSA-7gmj-67g7-phm9"
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

## Vulnerability Overview
A moderate-severity vulnerability (CVSS 6.1, CVE-2026-42184) exists in Tauri's `is_local_url()` function that incorrectly classifies remote URLs as trusted local origins on Windows and Android systems.

## Root Cause
On Windows and Android, Tauri maps custom URI schemes to `http://<scheme>.localhost/` format. The vulnerable validation logic uses `split_once('.')` which examines only the first subdomain, discarding everything after the initial dot. This allows an attacker to host a malicious page on a domain whose first label matches the application's custom scheme. For example, if the app registers an "app://" protocol, a page at `http://app.evil.com/` would be incorrectly classified as `Origin::Local`.

## Technical Details
The proper validation must verify the complete domain matches exactly `<protocol>.localhost`. The current implementation fails this check:

> "For `http://app.evil.com/`, the extracted label is app. If the application has registered a protocol named app, `protocols.contains_key("app")` returns true and the URL is classified as `Origin::Local`."

## Impact
Malicious remote pages can invoke backend commands restricted to local-only access, potentially allowing attackers to execute sensitive operations intended exclusively for the application's frontend.

## Affected and Patched Versions
- **Vulnerable:** Tauri ≥2.0, ≤2.11.0
- **Fixed:** Tauri ≥2.11.1

## Metadata
- **Severity:** Moderate (CVSS 6.1)
- **Affected Platforms:** Windows, Android
- **Reporter:** grumpinout1
- **Remediation Developer:** chippers
- **Reviewer:** FabianLars
- **Coordinator:** tweidinger
