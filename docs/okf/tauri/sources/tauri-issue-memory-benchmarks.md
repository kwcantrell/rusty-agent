---
type: Source
title: "Memory benchmark might be incorrect: Tauri might consume more RAM than Electron"
description: "GitHub issue #5889 challenging Tauri's memory benchmarks and reporting WebKit memory consumption discrepancies with Electron."
resource: https://github.com/tauri-apps/tauri/issues/5889
tags: [performance]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

GitHub issue #5889 challenges published Tauri memory benchmarks claiming Electron consumes ~500 MB on Linux, roughly double Tauri's usage. The reporter argues measurements fail to account for shared memory in Chromium-based applications.

**Key Technical Findings:**
- Tauri's WebKit implementation used substantially more RAM in real-world scenarios compared to Electron
- The difference exceeded 90 MB consistently across macOS, Ubuntu, and Windows
- USS and PSS metrics on Ubuntu showed narrower gaps than default measurements
- Benchmarking tool `mprof` supports multiple memory measurement backends (psutil, psutil_pss, psutil_uss)

**Critical Point:** WebKit-based applications appear to consume more memory than Chromium-based ones during typical web app usage, contradicting Tauri's positioning as more memory-efficient than Electron. The issue highlights that shared memory accounting fundamentally changes the memory efficiency comparison.
