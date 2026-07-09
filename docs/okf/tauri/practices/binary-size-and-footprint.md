---
type: Practice
title: Binary size and footprint
description: Shrink a Tauri binary with Cargo release-profile levers and unused-command removal — and read the Tauri-vs-Electron benchmarks skeptically, since WebKit's memory footprint can exceed Chromium's once shared memory is accounted for.
tags: [performance]
timestamp: 2026-07-09T00:00:00Z
---

# Binary size and footprint

Tauri produces small binaries by default — smaller than Electron by using the OS
system WebView instead of bundling one [1][2]. But "small by default" leaves real
headroom, and the framework's memory-efficiency reputation deserves a skeptical
read. Apply the size levers deliberately, and treat the published benchmarks as a
starting point rather than a settled claim.

## Cargo release-profile levers

The primary size control is the `[profile.release]` section of
`src-tauri/Cargo.toml` [1]. The documented levers, each a distinct trade-off [1]:

- `codegen-units = 1` — lets LLVM optimize across the whole crate (slower compile).
- `lto = true` — link-time optimization; trades compile time for a smaller binary.
- `opt-level = "s"` — prioritizes small size; use `3` instead if you want speed.
- `panic = "abort"` — disables unwinding/panic handlers, cutting runtime size.
- `strip = true` — removes debug symbols (do this for every release build).

On nightly, `trim-paths = "all"` additionally strips potentially sensitive build-
path information from the binary [1].

## Remove unused commands

Since Tauri 2.4, `removeUnusedCommands: true` in the `build` config eliminates
commands your app never uses from the binary [1]. It requires coordinated versions
— `tauri@2.4`, `tauri-build@2.1`, `tauri-plugin@2.1`, `tauri-cli@2.4` or later —
and works by reading your ACL (capability) files, so it pays off only if you
specify *only necessary* commands there rather than accepting plugin defaults [1].
That is the same discipline least-privilege security asks of you, so it lands as a
combined size-and-security win
([/practices/capabilities-permissions-scopes.md](/practices/capabilities-permissions-scopes.md)).

## Read the benchmarks skeptically

Tauri publishes a benchmark repository comparing Tauri, Wry, and Electron across
execution time, binary size, peak memory, thread count, and syscall count, run on
GitHub Actions across the three desktop OSes with `hyperfine` (3 warm-ups, 10
sequences) over three primary workloads: Hello World (startup time), CPU
Intensive (prime calculation with web workers), and Custom Protocol/File
Transfer (3MB files) [3]. Useful, but do not take the memory numbers at face
value. GitHub issue #5889 challenges the published memory results directly: the
methodology fails to account for shared memory in Chromium-based apps, and in
real-world use Tauri's WebKit implementation consumed **substantially more** RAM
than Electron — a gap exceeding 90 MB consistently across macOS, Ubuntu, and
Windows [4]. USS and PSS metrics on Ubuntu narrowed the gap versus the default
measurement, underscoring that *which* memory metric you pick changes the
conclusion [4]. The blunt takeaway: WebKit-based apps can consume more memory than
Chromium-based ones during typical web-app usage, contradicting the "more
memory-efficient than Electron" positioning [4].

The practical guidance: Tauri's binary-size advantage is real and directly
controllable with the profile levers above; its runtime *memory* advantage is not
a given and depends on the WebView engine and the metric — measure your own app
with a shared-memory-aware tool (USS/PSS) before making a memory claim [4]. Webview
choice is not yours to make per-platform — you inherit the OS engine — so budget
for WebKit's footprint on Linux and macOS rather than assuming the headline
numbers.

# Citations

1. [App Size Optimization](/sources/tauri-app-size.md)
2. [Beyond Electron: Attacking Alternative Desktop Application Frameworks](/sources/bishopfox-beyond-electron.md)
3. [Benchmark results](/sources/tauri-benchmark-results.md)
4. [Memory benchmark might be incorrect](/sources/tauri-issue-memory-benchmarks.md)
