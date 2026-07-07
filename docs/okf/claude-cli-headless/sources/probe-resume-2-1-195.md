---
type: Source
resource: https://github.com/anthropics/claude-code
---

# Probe: session resume round-trip — claude 2.1.195

Binary: `claude 2.1.195 (Claude Code)`  
Date: 2026-07-07

## Recipe

```bash
# Leg 1: plant codeword
cd "$SCRATCH/probes"
claude -p --output-format stream-json --verbose --allowedTools "" --model sonnet \
  --setting-sources project --strict-mcp-config \
  <<< "The codeword is umbrella. Reply OK." > resume1.jsonl 2>&1

# Extract session ID
SID=$(grep -o '"session_id":"[^"]*"' resume1.jsonl | head -1 | cut -d'"' -f4)

# Leg 2: resume and recall
claude -p --resume "$SID" --output-format stream-json --verbose --allowedTools "" --model sonnet \
  --setting-sources project --strict-mcp-config \
  <<< "What is the codeword? Answer with one word." > resume2.jsonl 2>&1
```

## Results

**Codeword recall:** `umbrella` was correctly recalled in resume2.jsonl. State carried.

**Session ID — same or new:** The resumed call's init event shows the same `session_id` as leg 1.  
`SID=81898c55-fde9-4d44-b1f5-fbd77cd4fef6`  
resume2 init: `"session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6"` — SAME.  
Implication: `--resume` reuses the original session ID; the client does not need to re-capture a new ID from the resumed init event, though re-capturing is harmless (Task 5 re-captures regardless).

**Cache evidence:** `cache_read_input_tokens: 22288` in resume2 (both assistant and result lines).

## Verbatim resume1.jsonl

```json
{"type":"system","subtype":"init","cwd":"/tmp/claude-1000/-home-kalen-rust-agent-runtime-43864113-1662-4940-8fca-c2a83c6a379f/scratchpad/probes","session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","tools":["Task","Bash","CronCreate","CronDelete","CronList","DesignSync","Edit","EnterWorktree","ExitWorktree","Monitor","NotebookEdit","PushNotification","Read","RemoteTrigger","ScheduleWakeup","SendMessage","Skill","TaskCreate","TaskGet","TaskList","TaskOutput","TaskStop","TaskUpdate","ToolSearch","WebFetch","WebSearch","Workflow","Write"],"mcp_servers":[],"model":"claude-sonnet-4-6","permissionMode":"default","slash_commands":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator","clear","compact","config","context","heapdump","init","reload-skills","review","security-review","usage-credits","extra-usage","usage","insights","goal","team-onboarding"],"apiKeySource":"none","claude_code_version":"2.1.195","output_style":"default","agents":["claude","Explore","general-purpose","Plan","statusline-setup"],"skills":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator"],"plugins":[],"analytics_disabled":false,"product_feedback_disabled":false,"uuid":"265882d9-52aa-4494-b88d-61521bfd86c0","memory_paths":{"auto":"/home/kalen/.claude/projects/-tmp-claude-1000--home-kalen-rust-agent-runtime-43864113-1662-4940-8fca-c2a83c6a379f-scratchpad-probes/memory/"},"fast_mode_state":"off"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","id":"msg_011CcoMtx3KD3YjwEV947Dei","type":"message","role":"assistant","content":[{"type":"text","text":"OK"}],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":6354,"cache_read_input_tokens":15934,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":6354},"output_tokens":1,"service_tier":"standard","inference_geo":"not_available"},"diagnostics":null,"context_management":null},"parent_tool_use_id":null,"session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","uuid":"816e0888-9ec8-445e-809f-ebc29571073b","request_id":"req_011CcoMtwM9Ho7VWT8xJUqjw"}
{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"duration_ms":1425,"duration_api_ms":2492,"ttft_ms":1397,"ttft_stream_ms":1281,"time_to_request_ms":29,"num_turns":1,"result":"OK","stop_reason":"end_turn","session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","total_cost_usd":0.043551200000000005,"usage":{"input_tokens":2,"cache_creation_input_tokens":6354,"cache_read_input_tokens":15934,"output_tokens":4,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":6354,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2,"output_tokens":4,"cache_read_input_tokens":15934,"cache_creation_input_tokens":6354,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":6354},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-haiku-4-5-20251001":{"inputTokens":511,"outputTokens":14,"cacheReadInputTokens":0,"cacheCreationInputTokens":0,"webSearchRequests":0,"costUSD":0.0005809999999999999,"contextWindow":200000,"maxOutputTokens":32000},"claude-sonnet-4-6":{"inputTokens":2,"outputTokens":4,"cacheReadInputTokens":15934,"cacheCreationInputTokens":6354,"webSearchRequests":0,"costUSD":0.04297020000000001,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","uuid":"c5f593b6-760a-44cc-bbee-d525f35ecc48"}
```

## Verbatim resume2.jsonl

```json
{"type":"system","subtype":"init","cwd":"/tmp/claude-1000/-home-kalen-rust-agent-runtime-43864113-1662-4940-8fca-c2a83c6a379f/scratchpad/probes","session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","tools":["Task","Bash","CronCreate","CronDelete","CronList","DesignSync","Edit","EnterWorktree","ExitWorktree","Monitor","NotebookEdit","PushNotification","Read","RemoteTrigger","ScheduleWakeup","SendMessage","Skill","TaskCreate","TaskGet","TaskList","TaskOutput","TaskStop","TaskUpdate","ToolSearch","WebFetch","WebSearch","Workflow","Write"],"mcp_servers":[],"model":"claude-sonnet-4-6","permissionMode":"default","slash_commands":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator","clear","compact","config","context","heapdump","init","reload-skills","review","security-review","usage-credits","extra-usage","usage","insights","goal","team-onboarding"],"apiKeySource":"none","claude_code_version":"2.1.195","output_style":"default","agents":["claude","Explore","general-purpose","Plan","statusline-setup"],"skills":["deep-research","design-sync","update-config","verify","debug","code-review","simplify","batch","fewer-permission-prompts","loop","schedule","claude-api","run","run-skill-generator"],"plugins":[],"analytics_disabled":false,"product_feedback_disabled":false,"uuid":"ef34f112-522d-4f11-833b-c71f718c65fb","memory_paths":{"auto":"/home/kalen/.claude/projects/-tmp-claude-1000--home-kalen-rust-agent-runtime-43864113-1662-4940-8fca-c2a83c6a379f-scratchpad-probes/memory/"},"fast_mode_state":"off"}
{"type":"assistant","message":{"model":"claude-sonnet-4-6","id":"msg_011CcoMucZpYsQN1JS3tHMir","type":"message","role":"assistant","content":[{"type":"text","text":"umbrella"}],"stop_reason":null,"stop_sequence":null,"stop_details":null,"usage":{"input_tokens":2,"cache_creation_input_tokens":19,"cache_read_input_tokens":22288,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":19},"output_tokens":1,"service_tier":"standard","inference_geo":"not_available"},"diagnostics":null,"context_management":null},"parent_tool_use_id":null,"session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","uuid":"cccb737a-3463-406a-8ddd-f32471a36533","request_id":"req_011CcoMubnwHT1HGZhXj7CZT"}
{"type":"result","subtype":"success","is_error":false,"api_error_status":null,"duration_ms":1342,"duration_api_ms":1316,"ttft_ms":1315,"ttft_stream_ms":1269,"time_to_request_ms":27,"num_turns":1,"result":"umbrella","stop_reason":"end_turn","session_id":"81898c55-fde9-4d44-b1f5-fbd77cd4fef6","total_cost_usd":0.0068814,"usage":{"input_tokens":2,"cache_creation_input_tokens":19,"cache_read_input_tokens":22288,"output_tokens":5,"server_tool_use":{"web_search_requests":0,"web_fetch_requests":0},"service_tier":"standard","cache_creation":{"ephemeral_1h_input_tokens":19,"ephemeral_5m_input_tokens":0},"inference_geo":"not_available","iterations":[{"input_tokens":2,"output_tokens":5,"cache_read_input_tokens":22288,"cache_creation_input_tokens":19,"cache_creation":{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":19},"type":"message"}],"speed":"standard"},"modelUsage":{"claude-sonnet-4-6":{"inputTokens":2,"outputTokens":5,"cacheReadInputTokens":22288,"cacheCreationInputTokens":19,"webSearchRequests":0,"costUSD":0.0068814,"contextWindow":200000,"maxOutputTokens":32000}},"permission_denials":[],"terminal_reason":"completed","fast_mode_state":"off","uuid":"2971b148-432c-49f3-a456-32a2852604ae"}
```

## Session file location

Session files are stored at:
```
~/.claude/projects/<cwd-slug>/<session_id>.jsonl
```
where `<cwd-slug>` is the working directory with path separators replaced by dashes.

Actual location for this probe (cwd was the scratchpad probes dir):
```
/home/kalen/.claude/projects/-tmp-claude-1000--home-kalen-rust-agent-runtime-43864113-1662-4940-8fca-c2a83c6a379f-scratchpad-probes/81898c55-fde9-4d44-b1f5-fbd77cd4fef6.jsonl
```

## Bogus resume failure

```bash
claude -p --resume 00000000-0000-0000-0000-000000000000 \
  --output-format json --no-session-persistence \
  --allowedTools "" --model sonnet <<< "hi"
```

Output (stderr):
```
No conversation found with session ID: 00000000-0000-0000-0000-000000000000
```

Exit code: `1`

The CLI exits immediately with a non-zero code and a plain-text error message on stderr. No JSON is emitted on stdout.
