# Practices

How this repo's `agent-model` claude_cli client uses the headless CLI surface.

- [delta-resume](delta-resume.md) — three-state session-reuse machine; persist on second use
- [prefix-invalidation](prefix-invalidation.md) — fingerprint-based extension check; reset on rewrite
- [auth-preservation](auth-preservation.md) — why `--bare` is omitted; `AGENT_API_KEY` removal; `--setting-sources`
- [stderr-drain](stderr-drain.md) — concurrent stderr drain to avoid ~64 KiB pipe-buffer deadlock
- [flag-pinning-tests](flag-pinning-tests.md) — proc-test pattern for each load-bearing CLI flag
