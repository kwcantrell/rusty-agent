---
type: Source
title: "tauri-apps/benchmark_results"
description: "Performance benchmark repository comparing Tauri, Wry, and Electron across execution time, binary size, memory usage, thread count, and syscall metrics."
resource: https://github.com/tauri-apps/benchmark_results
tags: [performance]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

The Tauri Benchmark Results repository stores comprehensive performance data comparing Tauri with Electron and Wry frameworks. Benchmarks measure:

**Performance Metrics:**
- Execution time via hyperfine (3 warm-up cycles, 10 test sequences)
- Binary size in release mode
- Memory usage at peak consumption
- Thread count during execution
- Syscall count for system calls

**Test Applications:** CPU-intensive prime number computation with web workers, hello world startup, custom protocol, and file transfer operations.

**Methodology:** Tests run on GitHub Actions across ubuntu-latest, windows-latest, and macos-latest. Results include execution time statistics (mean, standard deviation, user/system time, min/max), binary sizes, thread counts, and dependency metrics across platforms.
