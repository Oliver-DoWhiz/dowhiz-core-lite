FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates coreutils nodejs npm \
    && npm install -g @openai/codex \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

ENV CODEX_MODEL= \
    OPENAI_API_KEY= \
    OPENAI_BASE_URL= \
    AZURE_OPENAI_API_KEY= \
    AZURE_OPENAI_BASE_URL= \
    AZURE_OPENAI_MODEL=

COPY containers/codex-runner/configure_codex.sh /app/configure_codex.sh
COPY containers/codex-runner/entrypoint.sh /app/entrypoint.sh
COPY containers/codex-runner/exec_codex.sh /app/exec_codex.sh
RUN mkdir -p /root/.codex \
    && chmod +x /app/configure_codex.sh /app/entrypoint.sh /app/exec_codex.sh

ENTRYPOINT ["/app/entrypoint.sh"]
