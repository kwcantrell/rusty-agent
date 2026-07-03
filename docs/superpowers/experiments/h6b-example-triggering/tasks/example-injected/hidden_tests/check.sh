#!/usr/bin/env bash
# Hidden oracle for the H6b example-triggering task.
# Passes ONLY if report.md follows the STRICT house format that lives solely in
# the csv-report skill's worked example — so passing REQUIRES having read it.
set -u
f="report.md"

if [ ! -f "$f" ]; then
  echo "check: report.md missing" >&2
  exit 1
fi

# 1) Header line: exactly "# Report: <name>" (first non-empty line).
if ! grep -Eq '^# Report: .+' "$f"; then
  echo "check: missing '# Report:' header line" >&2
  exit 1
fi

# 2) Markdown table header present.
if ! grep -Eq '^\| *metric *\| *value *\|' "$f"; then
  echo "check: missing '| metric | value |' table header" >&2
  exit 1
fi

# 3) At least one value rendered to EXACTLY 2 decimals (e.g. 10.50, 7.00).
if ! grep -Eq '[0-9]+\.[0-9]{2}([^0-9]|$)' "$f"; then
  echo "check: no 2-decimal value found" >&2
  exit 1
fi

# 4) Final line: exactly "TOTAL: <sum-2dp>".
if ! grep -Eq '^TOTAL: [0-9]+\.[0-9]{2} *$' "$f"; then
  echo "check: missing/!2dp 'TOTAL:' final line" >&2
  exit 1
fi

echo "check: OK — strict house format present" >&2
exit 0
