FROM ubuntu:24.04

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash ca-certificates coreutils \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

COPY containers/codex-runner/entrypoint.sh /app/entrypoint.sh
COPY containers/codex-runner/exec_codex.sh /app/exec_codex.sh
RUN chmod +x /app/entrypoint.sh /app/exec_codex.sh

ENTRYPOINT ["/app/entrypoint.sh"]
