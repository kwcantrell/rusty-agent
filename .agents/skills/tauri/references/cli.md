# The `tauri` CLI

Invoke the CLI through your package manager (`npm run tauri <cmd>`,
`npx tauri <cmd>`, `pnpm tauri <cmd>`) or, if installed via Cargo,
`cargo tauri <cmd>`. All examples below use the bare `tauri <cmd>` form.

Pass arguments through to the underlying frontend tool with `--`:
`tauri dev -- --port 3000` forwards `--port 3000` to `beforeDevCommand`.

## Lifecycle

### `tauri dev`
Run the app in development. Runs `build.beforeDevCommand`, waits for
`build.devUrl`, launches the app, and hot-reloads.
- `-r, --release` — run the dev app in release profile.
- `--no-watch` — don't watch `src-tauri` for changes / don't recompile on change.
- `-f, --features <list>` — enable Cargo features.
- `-c, --config <json|path>` — merge an extra config (inline JSON or file).
- `--exit-on-panic`, `-- <args>` (forwarded to the before command).

### `tauri build`
Build the frontend (`build.beforeBuildCommand`), compile Rust in release, and
produce platform bundles. Output: `src-tauri/target/release/bundle/`.
- `-d, --debug` — build with the debug profile (debuggable, larger).
- `-b, --bundles <list>` — choose bundle targets, e.g. `deb,appimage`, `nsis`,
  `dmg`, `app`, or `none`.
- `--no-bundle` — compile the binary but skip packaging.
- `-t, --target <triple>` — cross/explicit Rust target triple.
- `-f, --features <list>`, `-c, --config <json|path>`.
- `-- <args>` forwarded to the before command.

## Project setup

### `tauri init`
Add Tauri (the `src-tauri/` directory + config) to an existing frontend project.
Prompts for app name, window title, `frontendDist`, and `devUrl`. Non-interactive
flags exist (`--app-name`, `--window-title`, `--frontend-dist`, `--dev-url`,
`--before-dev-command`, `--before-build-command`, `--ci`).

### `tauri add <plugin>`
Add an official plugin in one step: edits `Cargo.toml`, installs the JS package,
and registers boilerplate where possible. Example: `tauri add fs`, `tauri add
dialog`, `tauri add store`, `tauri add http`. You still grant the plugin's
permissions in a capabilities file (see `security.md`).

### `tauri migrate`
Migrate a Tauri **v1** project to **v2**: updates dependencies, rewrites config,
and converts the old `allowlist` into `capabilities`. Run once when upgrading.

## Diagnostics & assets

### `tauri info`
Print environment + project diagnostics: OS, Rust/Cargo versions, Node/pm
versions, `tauri`/`@tauri-apps/*` versions, WebView version, and detected
config. The first thing to run when debugging a broken setup.

### `tauri icon [input.png]`
Generate the full icon set (all platform sizes/formats) from a single square
source PNG (1024×1024 recommended) into `src-tauri/icons/`.

## Permissions & signing

### `tauri permission`
Manage ACL permissions. Subcommands include:
- `tauri permission ls [filter]` — list available permissions (e.g. for a plugin).
- `tauri permission add <identifier> [--capability <name>]` — add a permission to
  a capability file.
- `tauri permission rm <identifier>` — remove one.
- `tauri permission new <name>` — scaffold a custom permission.

### `tauri capability`
Scaffold/manage capability files (`tauri capability new <name>`).

### `tauri signer`
Updater key management:
- `tauri signer generate -w <path>` — generate an updater keypair (private key +
  public key). Keep the private key secret; put the public key in the updater
  plugin config.
- `tauri signer sign -k <private-key> <file>` — sign an artifact / produce the
  signature consumed by the updater.

## Plugin authoring

### `tauri plugin new <name>` / `tauri plugin init`
Scaffold a new Tauri plugin (Rust crate + JS API package + permission files), or
initialize plugin platform folders. Use when writing a **reusable** plugin rather
than app-local commands.

## Misc

- `tauri completions --shell <bash|zsh|fish|powershell>` — generate shell
  completions.
- `tauri android` / `tauri ios` — mobile targets (out of scope here).

## Config merging

`tauri.conf.json` is the base. Tauri merges, in order, platform-specific
overrides `tauri.<platform>.conf.json` (e.g. `tauri.linux.conf.json`,
`tauri.windows.conf.json`, `tauri.macos.conf.json`) and any `--config` value,
using **JSON Merge Patch (RFC 7396)**: objects merge deeply, and a `null` value
deletes a key. This lets you keep platform differences in separate files.
