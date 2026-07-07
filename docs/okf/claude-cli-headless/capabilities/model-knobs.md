---
type: Practice
tags: [claude-cli, capability]
---

# Model Knobs

`--effort` sets the effort level for the current invocation without persisting
it. The exact allowed-value list, confirmed by probing 2.1.195, is:
`low`, `medium`, `high`, `xhigh`, `max` [1]. Passing an unknown value produces
a warning on stderr (`Warning: Unknown --effort value 'banana' — ignoring it and
using the default effort. Valid values: low, medium, high, xhigh, max.`) and the
invocation succeeds (exit code 0) — it is a soft-warn, not a hard error [1].
Implementations that validate `--effort` input client-side should use this list
verbatim and treat unknown values as a warning, not a fatal condition.

`--fallback-model` enables automatic fallback to one or more alternative models
when the primary is unavailable (overloaded or retired). It accepts a
comma-separated list tried in order and overrides the persistent `fallbackModel`
setting for the invocation [2]. The flag is accepted without error in print
mode; in the probe the primary model (opus) was available, so no fallback
occurred, but the flag was confirmed grammatically valid [1].

`--betas` exists in the CLI flag surface to enable beta features. It was not
exercised in the 2.1.195 probes; no claims are made about its behavior here.

# Citations

1. [probe-model-knobs-2-1-195](/sources/probe-model-knobs-2-1-195.md)
2. [cli-reference](/sources/cli-reference.md)
