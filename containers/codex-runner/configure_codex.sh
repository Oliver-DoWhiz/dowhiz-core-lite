#!/usr/bin/env bash
set -euo pipefail

mkdir -p "${HOME:-/root}/.codex"

if [[ -n "${AZURE_OPENAI_API_KEY:-}" && -z "${OPENAI_API_KEY:-}" ]]; then
  export OPENAI_API_KEY="$AZURE_OPENAI_API_KEY"
fi

if [[ -n "${AZURE_OPENAI_BASE_URL:-}" && -z "${OPENAI_BASE_URL:-}" ]]; then
  export OPENAI_BASE_URL="$AZURE_OPENAI_BASE_URL"
fi

if [[ -n "${AZURE_OPENAI_MODEL:-}" && -z "${CODEX_MODEL:-}" ]]; then
  export CODEX_MODEL="$AZURE_OPENAI_MODEL"
fi

CONFIG_PATH="${HOME:-/root}/.codex/config.toml"

{
  printf 'model = "%s"\n' "${CODEX_MODEL:-gpt-5-codex}"
  if [[ -n "${OPENAI_BASE_URL:-}" ]]; then
    printf 'openai_base_url = "%s"\n' "$OPENAI_BASE_URL"
  fi
} >"$CONFIG_PATH"
