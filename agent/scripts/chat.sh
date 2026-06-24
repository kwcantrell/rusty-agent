#!/usr/bin/env bash
# chat.sh — poke an OpenAI-compatible chat server (llama.cpp / SGLang / vLLM) safely.
#
# Builds every request body with `jq` so a stray newline or quote in your prompt
# can never produce malformed JSON (the classic "500 parse error"). Can also show
# the *rendered prompt* via llama.cpp's POST /apply-template — the ground truth for
# chat-template behaviour (enable_thinking / preserve_thinking / reasoning_content
# / tool formatting), which a model's *answer* can't reliably reveal.
#
# Env (all optional):
#   BASE     server base url           (default http://localhost:8080)
#   MODEL    model id                  (default: first id from /v1/models)
#   TOKEN    bearer token              (default sk-noop; local servers ignore it)
#   TEMP     temperature               (default 0.2)
#   THINK    enable_thinking  1|0      (default 1)
#   PRESERVE preserve_thinking 1|0     (default 0)  -> Qwen3.6 keeps prior reasoning
#   RAW      1 => print full response JSON instead of a summary
#
# Commands:
#   chat.sh models                      list served model ids
#   chat.sh template [regex]            dump the loaded chat template (optionally grep it)
#   chat.sh ask "<prompt>"              one-shot user turn -> reasoning + content
#   chat.sh complete [msgs.json]        messages array (file or stdin) -> completion
#   chat.sh render   [msgs.json]        messages array (file or stdin) -> rendered prompt
#   chat.sh raw      [request.json]     full request body (file or stdin) -> completion
#   chat.sh vote <N> "<prompt>"         N samples, majority-vote the last line (self-consistency)
#
# Examples:
#   chat.sh ask "In one sentence, what is a chat template?"
#   PRESERVE=1 chat.sh render multiturn.json | grep PURPLE         # is prior reasoning kept?
#   THINK=0 chat.sh ask "Is 91 prime?"                             # thinking suppressed
#   chat.sh template preserve_thinking                             # does the template support it?
#   TEMP=0.8 chat.sh vote 10 "A bat and ball cost \$1.10 ... ball?"
set -euo pipefail

BASE="${BASE:-http://localhost:8080}"
AUTH="Authorization: Bearer ${TOKEN:-sk-noop}"
TEMP="${TEMP:-0.2}"
THINK="${THINK:-1}"
PRESERVE="${PRESERVE:-0}"
CT="Content-Type: application/json"

die()  { echo "error: $*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "need '$1' on PATH"; }
need jq; need curl

# Resolve a model id once (first /v1/models entry) unless the caller pinned MODEL.
model() {
  if [[ -z "${MODEL:-}" ]]; then
    MODEL="$(curl -sS -m 10 "$BASE/v1/models" -H "$AUTH" | jq -r '.data[0].id // empty')"
    [[ -n "$MODEL" ]] || die "no model id at $BASE/v1/models — set MODEL=…"
  fi
  printf '%s' "$MODEL"
}

jbool() { [[ "$1" == 1 ]] && echo true || echo false; }

# stdin: JSON messages array -> stdout: full chat request with our flags applied.
wrap() {
  jq -n \
    --arg m "$(model)" \
    --argjson msgs "$(cat)" \
    --argjson temp "$TEMP" \
    --argjson think "$(jbool "$THINK")" \
    --argjson preserve "$(jbool "$PRESERVE")" \
    '{model:$m, messages:$msgs, temperature:$temp,
      chat_template_kwargs:{enable_thinking:$think, preserve_thinking:$preserve}}'
}

# $1 = path; stdin: body -> stdout: response body (surfaces server .error to stderr).
post() {
  local out
  out="$(curl -sS -m 300 "$BASE$1" -H "$AUTH" -H "$CT" --data-binary @-)"
  if jq -e 'has("error")' >/dev/null 2>&1 <<<"$out"; then
    { echo "server error from $1:"; jq '.error' <<<"$out"; } >&2
    return 1
  fi
  printf '%s' "$out"
}

# stdin: chat.completion json -> compact summary (or full json if RAW=1).
show() {
  if [[ "${RAW:-0}" == 1 ]]; then jq .; else
    jq '{finish: .choices[0].finish_reason,
         reasoning: .choices[0].message.reasoning_content,
         content:   .choices[0].message.content,
         usage}'
  fi
}

src() { if [[ -n "${1:-}" ]]; then cat -- "$1"; else cat; fi; }  # file arg or stdin

cmd="${1:-help}"; shift || true
case "$cmd" in
  models)
    curl -sS -m 10 "$BASE/v1/models" -H "$AUTH" | jq -r '.data[].id' ;;
  template)
    curl -sS -m 10 "$BASE/props" -H "$AUTH" \
      | jq -r '.chat_template // .default_generation_settings.chat_template // empty' \
      | { if [[ -n "${1:-}" ]]; then grep -nE "$1" || echo "(no match for /$1/)"; else cat; fi; } ;;
  ask)
    [[ -n "${1:-}" ]] || die 'usage: chat.sh ask "<prompt>"'
    jq -n --arg c "$1" '[{role:"user",content:$c}]' | wrap | post /v1/chat/completions | show ;;
  complete)
    src "${1:-}" | wrap | post /v1/chat/completions | show ;;
  render)
    src "${1:-}" | wrap | post /apply-template | jq -r '.prompt // .' ;;
  raw)
    src "${1:-}" | post /v1/chat/completions | show ;;
  vote)
    n="${1:-5}"; shift || true
    [[ -n "${1:-}" ]] || die 'usage: chat.sh vote <N> "<prompt>"'
    : "${TEMP:=0.8}"
    for _ in $(seq "$n"); do
      jq -n --arg c "$1" '[{role:"user",content:$c}]' | wrap \
        | post /v1/chat/completions | jq -r '.choices[0].message.content | split("\n") | last'
    done | sort | uniq -c | sort -rn ;;
  help|--help|-h|*)
    sed -n '2,33p' "$0" ;;
esac
