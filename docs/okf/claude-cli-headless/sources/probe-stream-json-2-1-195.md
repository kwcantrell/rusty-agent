---
type: Source
resource: https://github.com/anthropics/claude-code
---

# Probe: stream-json output shape — claude 2.1.195

Binary: `claude 2.1.195 (Claude Code)`  
Date: 2026-07-07  
Command: `claude -p --output-format stream-json --verbose --include-partial-messages --allowedTools "" --model sonnet --setting-sources project --strict-mcp-config --no-session-persistence <<< "Say exactly: hello"`

## Captured lines (Steps 1 & 2)

### init line

```json
{"type":"system","subtype":"init","cwd":"/home/kalen/rust-agent-runtime","session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","tools":["Task","Bash","CronCreate","CronDelete","CronList","DesignSync","Edit","EnterWorktree","ExitWorktree","Monitor","NotebookEdit","PushNotification","Read","RemoteTrigger","ScheduleWakeup","SendMessage","Skill","TaskCreate","TaskGet","TaskList","TaskOutput","TaskStop","TaskUpdate","ToolSearch","WebFetch","WebSearch","Workflow","Write"],"mcp_servers":[],"model":"claude-sonnet-4-6","permissionMode":"default","slash_commands":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator","clear","compact","config","context","heapdump","init","reload-skills","review","security-review","usage-credits","extra-usage","usage","insights","goal","team-onboarding"],"apiKeySource":"none","claude_code_version":"2.1.195","output_style":"default","agents":["claude","Explore","general-purpose","Plan","statusline-setup"],"skills":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator"],"plugins":[],"analytics_disabled":false,"product_feedback_disabled":false,"uuid":"9f605e3a-53d6-431f-a75b-27e396f377be","memory_paths":{"auto":"/home/kalen/.claude/projects/-home-kalen-rust-agent-runtime/memory/"},"fast_mode_state":"off"}
```

Key init fields: `type: "system"`, `subtype: "init"`, `session_id`, `model` (resolved model id, e.g. `"claude-sonnet-4-6"`), `claude_code_version: "2.1.195"`, `tools` array, `mcp_servers` array.

### message_start stream_event

```json
{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-4-6","id":"msg_011CcoMohKeVVbHLgCVVjcDy","type":"message","role":"assistant","content":[],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"output_tokens":1,"service_tier":"standard","inference_geo":"not_available"},"diagnostics":null}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"7a48fa8b-9dea-415d-9378-cafff61a58fd","ttft_ms":1978}
```

### content_block_start stream_event

```json
{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"88067952-3788-4b78-9ca1-9b813ed478f3"}
```

### text_delta stream_event

```json
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"f7612ac4-3352-4d69-bdd4-deccecc0ee2b"}
```

Delta path: `.event.delta.type == "text_delta"`, text at `.event.delta.text`.

### assistant (whole message, emitted mid-stream with --include-partial-messages)

```json
{"type":"assistant","message":{"model":"claude-sonnet-4-6","id":"msg_011CcoMohKeVVbHLgCVVjcDy","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"output_tokens":1,"service_tier":"standard","inference_geo":"not_available"},"diagnostics":null,"context_management":null},"parent_tool_use_id":null,"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","uuid":"4a79959d-3a36-4656-8d9c-01dd3645f578","request_id":"req_011CcoMogMrNdDwSc8fZCS1c"}
```

### message_delta stream_event (with final usage)

```json
{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null,"stop_details":null},"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"output_tokens":4,"output_tokens_details":{"thinking_tokens":0},"iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":20333,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"type":"message"}]},"context_management":{"applied_edits":[]}},"session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","parent_tool_use_id":null,"uuid":"afe28b9a-d8a5-4a84-9851-f9bc63aa4fee"}
```

### result line

```json
{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"duration_ms":2118,"duration_api_ms":3091,"ttft_ms":2085,"ttft_stream_ms":2000,"time_to_request_ms":22,"num_turns":1,"result":"hello","stop_reason":"end_turn","session_id":"c33cee68-7fa0-4926-b32e-c1ad457673ea","total_cost_usd":0.122623,"usage":{"input_tokens":2,"cache_creation_input_tokens":20333,"cache_read_input_tokens":0,"output_tokens":4,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":20333,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":0,"cache_creation_input_tokens":20333,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":20333},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-haiku-4-5-20251001":{"inputTokens":504,"outputTokens":11,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.000559,"contextWindow":200000,"maxOutputTokens":32000},"claude-sonnet-4-6":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":0,"cacheCreationInputTokens":20333,"webSearchRequests":0,"costUSD":0.12206399999999999,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","uuid":"cf354c70-9f45-4d4c-9192-c87b5149b7bd"}
```

Result fields: `type: "result"`, `subtype: "success"`, `result` (final text), `session_id`, `usage` (with full token breakdown), `total_cost_usd`, `modelUsage` per-model breakdown.

## Dedup observation

The `assistant` message (type `"assistant"`) is emitted mid-stream (with `--include-partial-messages`) AND its `content[0].text` field contains the same text as the accumulated `text_delta` events. In this run the delta emitted `"hello"` and the assistant message contained `content: [{"type":"text","text":"hello"}]`. The whole assistant message DOES repeat / duplicate the text from all deltas — downstream consumers must choose one source (deltas OR whole message) to avoid double-counting. The `result.result` field also carries the same final text as a plain string.

## Thinking output (Step 2)

Probed with `--model opus --effort high` then retried with `--model sonnet --effort high`. Neither probe emitted `thinking_delta` stream events or a `thinking` content block. `output_tokens_details.thinking_tokens` was `0` in both runs. **No thinking emitted at this tier/config with claude 2.1.195.** Task 3 retains the parser arms based on the Anthropic SSE specification; the expected shapes when thinking IS enabled are:

- Stream event: `{"type":"stream_event","event":{"type":"content_block_delta","index":N,"delta":{"type":"thinking_delta","thinking":"...partial..."}}, ...}`
- Whole block: `{"type":"thinking","thinking":"...full reasoning..."}` inside `content[]` of the assistant message
