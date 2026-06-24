# Structured output & tool calling

Three escalating ways to make the model emit machine-parseable output, plus
function/tool calling and reasoning control.

## 1. GBNF grammars (most precise)

A GBNF grammar constrains generation token-by-token to a formal grammar — the
model *cannot* produce output the grammar forbids. Use it for strict formats
(enums, fixed shapes, DSLs) where even valid-JSON-but-wrong-shape is unacceptable.

Pass it server-wide with `--grammar`, or per-request with the `grammar` field on
`/completion` (or `/v1/chat/completions` via `response_format` below).

```bash
curl http://localhost:8080/completion -H 'Content-Type: application/json' -d '{
  "prompt": "Is the sky blue? ",
  "grammar": "root ::= \"yes\" | \"no\""
}'
```

## 2. JSON-schema-constrained output (most ergonomic)

Give a JSON Schema and the server compiles it to a grammar automatically.

- Server-wide: `-j '{"type":"object", ...}'` / `--json-schema`.
- Native per-request: `json_schema` field on `/completion`.
- OpenAI per-request: `response_format` on `/v1/chat/completions`:
  - `{"type":"json_object"}` — any valid JSON.
  - `{"type":"json_schema","json_schema":{"name":"x","schema":{...}}}` — schema-constrained.

```bash
curl http://localhost:8080/v1/chat/completions -H 'Content-Type: application/json' -d '{
  "messages":[{"role":"user","content":"Give me a person."}],
  "response_format":{"type":"json_schema","json_schema":{"name":"person","schema":{
    "type":"object",
    "properties":{"name":{"type":"string"},"age":{"type":"integer"}},
    "required":["name","age"]
  }}}
}'
```

> **Caveat — it's a subset.** The JSON-Schema → GBNF converter supports a
> *subset* of JSON Schema. Common keywords (`type`, `properties`, `required`,
> `enum`, `items`, basic `string`/`number`/`integer`/`boolean`/`array`/`object`)
> work; advanced constructs (`$ref` across files, `allOf`/`oneOf` combinations,
> complex `pattern`/`format` validation) may be ignored or rejected. Keep schemas
> simple; verify the output actually conforms.

## 3. Tool / function calling

OpenAI-style function calling: declare `tools`, the model replies with
`tool_calls`, you execute them and feed results back.

> **Requires `--jinja` at server startup.** It's default-on in current builds
> (was off in older ones). Without a correct Jinja chat template the server
> returns `finish_reason:"stop"` and ignores `tools` entirely.

```bash
curl http://localhost:8080/v1/chat/completions -H 'Content-Type: application/json' -d '{
  "messages":[{"role":"user","content":"What is the weather in Paris?"}],
  "tools":[{"type":"function","function":{
    "name":"get_weather",
    "description":"Get current weather for a city",
    "parameters":{"type":"object","properties":{"city":{"type":"string"}},"required":["city"]}
  }}],
  "tool_choice":"auto"
}'
```

The response contains `choices[0].message.tool_calls[]`, each with
`function.name` and `function.arguments` (a JSON **string**). Run the tool, then
send a follow-up turn with a `{"role":"tool","tool_call_id":...,"content":...}`
message.

**Template handling.** llama.cpp ships native tool-call parsers for major model
families — Llama 3.1/3.2/3.3, Functionary, Hermes 2/3, Qwen 2.5, Mistral Nemo,
Firefunction v2, Command R7B, DeepSeek R1 — and falls back to a **Generic**
handler otherwise. If tool calls come back malformed or never appear, the model's
template is the usual culprit (see `troubleshooting.md`); inspect the rendered
prompt with `POST /apply-template`.

> **Compat note (Rust/strict clients):** some older server builds emitted
> `function.arguments` as a JSON *object* instead of a *string*, breaking strict
> deserializers (llama.cpp issue #20198, fixed by PR #20213). Verify your build,
> or parse defensively.

## 4. Reasoning / "thinking" models

For models that emit chain-of-thought (DeepSeek-R1, QwQ, etc.):

- `-rea, --reasoning [on|off|auto]` — enable/disable reasoning tags (auto-detects).
- `--reasoning-format {none|deepseek|legacy}` — how thought tags are surfaced.
- `--reasoning-budget N` — token budget for thinking (`-1` = unrestricted).

Reasoning text is returned separately as `reasoning_content` in the response
(kept out of `content`). To stop thinking early on an in-flight request, POST to
`/v1/chat/completions/control` with `{"id":<completion-id>,"action":"reasoning_end"}`.
Some templates also toggle thinking via `--chat-template-kwargs`.
