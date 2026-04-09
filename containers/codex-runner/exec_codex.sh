#!/usr/bin/env bash
set -euo pipefail

WORKSPACE_DIR="${TASK_WORKSPACE_DIR:-/workspace}"
PROMPT_FILE="${TASK_PROMPT_FILE:-${WORKSPACE_DIR}/task_prompt.txt}"
OUTPUT_FILE="${TASK_OUTPUT_FILE:-${WORKSPACE_DIR}/.task_stdout.log}"
METADATA_FILE="${TASK_METADATA_FILE:-${WORKSPACE_DIR}/workspace_manifest.json}"

mkdir -p "$(dirname "$OUTPUT_FILE")"

if command -v codex >/dev/null 2>&1; then
  (
    cd "$WORKSPACE_DIR"
    codex exec --input-file "$PROMPT_FILE" >"$OUTPUT_FILE"
  )
  exit 0
fi

cat >"$OUTPUT_FILE" <<EOF
Codex CLI is not installed in this image.
This container is a contract stub for the lightweight replica.
Install Codex in the image to execute real tasks.

Workspace dir: $WORKSPACE_DIR
Prompt file: $PROMPT_FILE
Metadata file: $METADATA_FILE
EOF
