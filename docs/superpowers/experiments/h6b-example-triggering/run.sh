#!/usr/bin/env bash
# H6b example-triggering runner. Runs the live eval harness N times for one arm and
# reports trigger-rate (fraction of runs with gold_matched==true) and pass-rate
# (fraction that passed). Directional smoke tool — not a verdict.
#
# Usage: run.sh [N] [arm]
#   N   number of repeat runs (default 3)
#   arm one of: model-initiative | example-injected  (default model-initiative)
set -u

N="${1:-3}"
ARM="${2:-model-initiative}"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../../.." && pwd)"        # docs/superpowers/experiments/h6b-... -> repo root
TASK_DIR="$HERE/tasks/$ARM"
TASK_JSON="$TASK_DIR/task.json"
HIDDEN_TESTS_DIR="$TASK_DIR/hidden_tests"
CONFIG_JSON="$HERE/config.json"
SKILLS_DIR="$HERE/skills"

if [ ! -f "$TASK_JSON" ]; then
  echo "no such arm: $ARM (looked for $TASK_JSON)" >&2
  exit 2
fi

# NB: the OpenAiCompatClient appends `/v1/chat/completions`, so the base URL must
# NOT already end in /v1 (else it doubles to /v1/v1/... -> 404, swallowed silently).
export AGENT_E2E_URL="${AGENT_E2E_URL:-http://localhost:8080}"
export AGENT_E2E_MODEL="${AGENT_E2E_MODEL:-qwen3.6-35b-a3b}"
export SKILLS_DIR TASK_JSON CONFIG_JSON HIDDEN_TESTS_DIR

echo "== arm=$ARM  N=$N  url=$AGENT_E2E_URL  model=$AGENT_E2E_MODEL ==" >&2
echo "   SKILLS_DIR=$SKILLS_DIR" >&2

triggered=0
passed=0
done_runs=0

for i in $(seq 1 "$N"); do
  echo "--- run $i/$N ($ARM) ---" >&2
  # Capture stdout; the harness prints exactly one RunResult JSON line to stdout.
  out="$(cd "$REPO_ROOT/agent" && cargo test -p agent-runtime-config --test eval_context \
        eval_context_run -- --ignored --nocapture 2>>/tmp/h6b_stderr.$$ )"
  # The RunResult is the single stdout line that is a JSON object with "passed".
  line="$(printf '%s\n' "$out" | grep -E '^\{.*"passed".*\}$' | tail -n 1)"
  if [ -z "$line" ]; then
    echo "run $i: NO RunResult line captured (see /tmp/h6b_stderr.$$)" >&2
    printf '%s\n' "$out" | tail -n 20 >&2
    continue
  fi
  echo "$line"
  done_runs=$((done_runs + 1))
  # Parse with python for robustness.
  gm="$(printf '%s' "$line" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(1 if d.get("gold_matched") is True else 0)')"
  ps="$(printf '%s' "$line" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(1 if d.get("passed") is True else 0)')"
  triggered=$((triggered + gm))
  passed=$((passed + ps))
done

echo "== summary arm=$ARM ==" >&2
if [ "$done_runs" -eq 0 ]; then
  echo "arm=$ARM: 0 completed runs — see stderr log /tmp/h6b_stderr.$$" >&2
  exit 1
fi
python3 - "$ARM" "$done_runs" "$triggered" "$passed" "$N" <<'PY'
import sys
arm, done, trig, passed, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
tr = trig / done
pr = passed / done
print(f"arm={arm} runs_completed={done}/{n} trigger_rate={tr:.2f} ({trig}/{done}) pass_rate={pr:.2f} ({passed}/{done})")
PY
