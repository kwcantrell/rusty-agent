---
type: Source
title: "App Size Optimization in Tauri"
description: "Binary size reduction through Cargo profile configuration, link-time optimization, and unused command removal."
resource: https://v2.tauri.app/concept/size/
tags: [performance]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri provides small binaries by default. Further optimization through Cargo profile configuration in `src-tauri/Cargo.toml` release profile:

- **`codegen-units = 1`**: "Allows LLVM to perform better optimization"
- **`lto = true`**: "Enables link-time-optimizations"
- **`opt-level = "s"`**: "Prioritizes small binary size" (alternatively `3` for speed preference)
- **`panic = "abort"`**: "Higher performance by disabling panic handlers"
- **`strip = true`**: "Ensures debug symbols are removed"

For nightly toolchain, additional option: `trim-paths = "all"` removes potentially sensitive build information.

## Unused Command Removal

Tauri 2.4+ introduced feature to eliminate unused commands:

```json
{
  "build": {
    "removeUnusedCommands": true
  }
}
```

Requires coordination between tauri-cli, tauri-build, tauri-plugin, and capability files. Guidance emphasizes specifying only necessary commands in ACL files rather than using defaults.

**Version requirement**: tauri@2.4, tauri-build@2.1, tauri-plugin@2.1, tauri-cli@2.4 or later.

## Key Technical Details

- LLVM optimization improves with single codegen unit
- Link-time optimization trades compile time for smaller binaries
- Small size optimization (`opt-level = "s"`) vs speed optimization (`3`)
- Panic abort disables unwinding, reducing runtime size
- Debug symbol stripping mandatory for release
- Nightly-only trim-paths removes build path information
- ACL file configuration determines included commands
- Feature introduced in Tauri 2.4 series
