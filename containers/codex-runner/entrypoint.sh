#!/usr/bin/env bash
set -euo pipefail

/app/configure_codex.sh

if [[ $# -gt 0 ]]; then
  exec "$@"
fi

if [[ "${CODEX_RUNNER_MODE:-one_shot}" == "pool" ]]; then
  while true; do
    sleep 3600
  done
fi

exec /app/exec_codex.sh
