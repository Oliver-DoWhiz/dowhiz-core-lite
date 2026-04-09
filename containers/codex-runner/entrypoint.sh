#!/usr/bin/env bash
set -euo pipefail

PROMPT_FILE="${TASK_PROMPT_FILE:-/workspace/task_prompt.txt}"
OUTPUT_FILE="${TASK_OUTPUT_FILE:-/workspace/.task_stdout.log}"

if command -v codex >/dev/null 2>&1; then
  codex exec --input-file "$PROMPT_FILE" >"$OUTPUT_FILE"
  exit 0
fi

cat >"$OUTPUT_FILE" <<'EOF'
Codex CLI is not installed in this image.
This container is a contract stub for the lightweight replica.
Install Codex in the image or disable container mode for local simulation.
EOF
