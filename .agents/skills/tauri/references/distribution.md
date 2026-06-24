# Distribution: bundling, signing, updating (desktop)

`tauri build` compiles the release binary and packages installers into
`src-tauri/target/release/bundle/<target>/`. You can only build a platform's
installers **on that platform** (no cross-OS bundling for installers).

## Bundle targets per OS

| OS | Formats | Notes |
|----|---------|-------|
| **Windows** | `.msi` (WiX v3), `.exe` (NSIS) | NSIS supports more customization & per-user installs; MSI is enterprise-friendly. |
| **macOS** | `.app`, `.dmg` | `.app` is the bundle; `.dmg` is the distributable disk image. |
| **Linux** | `.deb`, `.rpm`, `.AppImage` | AppImage is the portable single-file option; deb/rpm integrate with package managers. |

Select targets with `tauri build --bundles <list>` or `bundle.targets` in config.
`--no-bundle` builds just the executable.

## Code signing — macOS

Unsigned macOS apps are blocked by Gatekeeper. You need an Apple Developer
account and a **Developer ID Application** certificate, then sign + **notarize**.
Tauri reads these from environment variables during `tauri build`:

- `APPLE_CERTIFICATE` (base64 of the `.p12`) + `APPLE_CERTIFICATE_PASSWORD`, or a
  keychain identity via `APPLE_SIGNING_IDENTITY`.
- Notarization: either `APPLE_API_ISSUER` + `APPLE_API_KEY` (+ key file) for App
  Store Connect API auth, **or** `APPLE_ID` + `APPLE_PASSWORD` (app-specific
  password) + `APPLE_TEAM_ID`.

`bundle.macOS.signingIdentity` / `entitlements` can be set in config. Tauri
notarizes automatically when notarization credentials are present.

## Code signing — Windows

Sign with a code-signing certificate (OV or EV; EV/cloud HSM is increasingly
required by SmartScreen). Options:

- Set `bundle.windows.certificateThumbprint` (cert installed in the Windows cert
  store) plus optional `digestAlgorithm` and `timestampUrl`.
- Or configure a custom sign command via `bundle.windows.signCommand` for cloud
  HSM / Azure Trusted Signing setups.

Signing happens during `tauri build`. A timestamp URL keeps signatures valid past
certificate expiry.

## Code signing — Linux

Linux distribution generally doesn't require OS-level code signing. For trust you
typically rely on the package repo / checksums, and the **updater** uses Tauri's
own signature scheme (below) regardless of OS.

## The updater (cross-platform)

The `updater` plugin checks an endpoint and applies signed updates. Setup:

1. `tauri add updater`.
2. Generate a keypair: `tauri signer generate -w ~/.tauri/myapp.key`. Keep the
   private key secret (CI secret); put the **public** key in
   `plugins.updater.pubkey`.
3. Set `plugins.updater.endpoints` (supports `{{target}}`, `{{arch}}`,
   `{{current_version}}` placeholders) and enable
   `bundle.createUpdaterArtifacts: true`.
4. On build, Tauri produces update artifacts; sign them (CI signs with the
   private key, e.g. `TAURI_SIGNING_PRIVATE_KEY` env). The endpoint serves a JSON
   manifest pointing at the signed artifact.
5. Frontend/Rust calls the updater API to `check()` and `downloadAndInstall()`.

Only updates signed with the matching private key are accepted — this is
independent of OS code signing and is what prevents malicious updates.

## Tips

- Bump `version` (config or `Cargo.toml`) every release; the updater compares it.
- Set a real `identifier`, `productName`, `icon`, and `category` before shipping.
- Test installers on a clean machine/VM — missing runtime deps (e.g. WebView2 on
  older Windows, webkit libs on Linux) only show up there.
- CI: build each OS on its own runner; inject signing secrets via environment
  variables, never commit keys.
