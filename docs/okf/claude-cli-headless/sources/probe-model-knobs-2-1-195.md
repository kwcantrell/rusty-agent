---
type: Source
resource: https://github.com/anthropics/claude-code
---

# Probe: --effort and --fallback-model knobs — claude 2.1.195

Binary: `claude 2.1.195 (Claude Code)`  
Date: 2026-07-07

## --effort allowed values

Command:
```bash
claude -p --effort banana --output-format json --no-session-persistence <<< "hi" 2>&1 | head -5
```

Output:
```
Warning: Unknown --effort value 'banana' — ignoring it and using the default effort. Valid values: low, medium, high, xhigh, max.
```

**Exact allowed-value list (verbatim from warning):** `low, medium, high, xhigh, max`

Important behavior note: this is a **warning**, not a hard error. The CLI accepts the flag, ignores the invalid value, falls through to the default effort level, and the invocation succeeds (exit code 0). Task 6's `EFFORT_LEVELS` validation constant should use this list verbatim: `["low", "medium", "high", "xhigh", "max"]`.

## --fallback-model acceptance

Command:
```bash
claude -p --fallback-model sonnet --output-format json --no-session-persistence \
  --allowedTools "" --model opus --setting-sources project --strict-mcp-config <<< "Say hi" 2>&1 | tail -3
```

Result: succeeded (exit code 0). The `--fallback-model` flag is accepted in print mode without error. The flag was not triggered in this probe (opus was available), so no actual fallback occurred — it is accepted as a flag configuration that would activate on primary model unavailability.

Full result line captured:
```json
{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"duration_ms":3081,"duration_api_ms":3059,"ttft_ms":3056,"ttft_stream_ms":2674,"time_to_request_ms":23,"num_turns":1,"result":"Hi! 👋 How can I help you with the rust-agent-runtime today?","stop_reason":"end_turn","session_id":"91bddcfd-5e31-437e-aa14-b0092fb1179f","total_cost_usd":0.0822255,"usage":{"input_tokens":2500,"cache_creation_input_tokens":6093,"cache_read_input_tokens":16191,"output_tokens":28,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":6093,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2500,"output_tokens":28,"cache_read_input_tokens":16191,"cache_creation_input_tokens":6093,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":6093},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-opus-4-8":{"inputTokens":2500,"outputTokens":28,"cacheReadInputTokens":16191,"cacheCreationInputTokens":6093,"webSearchRequests":0,"costUSD":0.0822255,"contextWindow":1000000,"maxOutputTokens":64000}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","uuid":"34c78cff-9fa4-44d9-837a-60e4eb5f6c5a"}
```
