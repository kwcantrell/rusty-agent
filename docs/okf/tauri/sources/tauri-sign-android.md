---
type: Source
title: "Android Code Signing | Tauri"
description: "Guide to digitally signing Android App Bundles and APKs for Play Store distribution using keytool and Gradle configuration."
resource: https://v2.tauri.app/distribute/sign/android/
tags: [mobile]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Distribution on Play Store requires signing applications with digital certificates: "Android App Bundles and APKs must be signed before being uploaded for distribution."

**Keystore Generation:**
Use Java's `keytool` CLI to create keystore file with example: `keytool -genkey -v -keystore ~/upload-keystore.jks -keyalg RSA -keysize 2048 -validity 10000 -alias upload`

**Configuration File:**
Create `keystore.properties` in `[project]/src-tauri/gen/android/` with password, key alias, and keystore file location.

**Gradle Integration:**
- Add import: `import java.io.FileInputStream`
- Configure signing configs in `app/build.gradle.kts` file
- Apply release signing configuration to buildTypes block

**Security:** Keep keystore files and properties files private; avoid committing to public source control. Many developers generate these in CI/CD pipelines rather than storing locally to minimize exposure risk.
