---
type: Practice
tags: [claude-cli, practice]
---

# Stderr Drain

The CLI emits diagnostic and error output on stderr while writing stream-json
events on stdout. The bogus-resume probe demonstrates a case where stderr carries
substantive output (`No conversation found with session ID: ...`) and stdout
carries nothing [1]. In normal operation, `--verbose` may produce additional
progress text on stderr alongside the JSON stream on stdout.

When the client pipes stdin to the CLI process while also reading stdout, a
deadlock risk exists at the OS pipe buffer boundary (~64 KiB on Linux). If the
CLI fills its stderr or stdout pipe buffer and the client is not consuming that
pipe, the CLI blocks; if the client is waiting for the CLI to consume stdin
before reading output, both sides stall. The safe pattern is:

1. Feed stdin on a separate async task that closes the write end of the pipe as
   soon as the input is fully written.
2. Drain stdout and stderr concurrently on separate tasks, buffering or
   discarding until EOF on both.

This ensures no pipe buffer can fill while the other side waits, regardless of
output volume.

# Citations

1. [probe-resume-2-1-195](/sources/probe-resume-2-1-195.md)
