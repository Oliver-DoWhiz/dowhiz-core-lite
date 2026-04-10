# DoWhiz Core Lite

`DoWhiz Core Lite` is a lightweight replica of the core DoWhiz backend architecture.
It keeps the essential runtime path:

```text
inbound gateway -> file queue -> worker -> run_task_module -> reply draft
```

This repo is intentionally small. It focuses on the parts that define the product's
core execution model and leaves out channel-specific integrations, auth surfaces,
billing, analytics, and legacy product layers.

For a direct explanation of the technical debt found in the upstream repo and how
this lightweight version addresses it, see [`TECHNICAL_DEBT.md`](TECHNICAL_DEBT.md).
For the scaling and runtime design notes added in response to issue feedback, see
[`docs/scaling_and_runtime.md`](docs/scaling_and_runtime.md).

## Why this repo exists

The upstream `KnoWhiz/DoWhiz` repository is powerful, but the core Rust service has
grown broad enough that the scheduler and worker paths are harder to evolve than they
need to be. This repo demonstrates a trimmed architecture with:

- One ingress binary: `inbound_gateway`
- One worker binary: `rust_service`
- One queue abstraction with a local file-backed implementation
- One task runner crate with a container boundary for Codex-style execution
- One outbound email preview crate
- One top-level technical debt summary plus a detailed audit writeup

## Layout

- `scheduler_module/`: ingress, queue, task workspace creation, worker loop
- `run_task_module/`: local runner and container runner for Codex-style execution
- `send_emails_module/`: outbound preview builder
- `containers/codex-runner/`: example container contract for task execution
- `TECHNICAL_DEBT.md`: direct summary of the technical debt and how this repo responds
- `docs/inefficiencies_solved.md`: audit summary and design rationale

## Quick start

```bash
cp .env.example .env
```

Set `OPENAI_API_KEY` in `.env`. If the `codex` CLI is installed locally, the worker
will now invoke it automatically without requiring `LOCAL_AGENT_COMMAND`.

Start the worker:

```bash
cargo run -p scheduler_module --bin rust_service
```

In another terminal, start the gateway:

```bash
cargo run -p scheduler_module --bin inbound_gateway
```

Create a task:

```bash
curl -X POST http://127.0.0.1:9100/tasks \
  -H 'content-type: application/json' \
  -d '{
    "customer_email": "dtang04@uchicago.edu",
    "subject": "Audit request",
    "prompt": "Analyze the repo and draft a reply.",
    "reply_to": "dtang04@uchicago.edu"
  }'
```

The worker writes per-task artifacts under `dowhiz-core-lite/.workspace/tasks/`.

## Frontend proof of concept

The repo now includes a small JavaScript frontend under
`scheduler_module/frontend/`. It submits JSON to `POST /tasks` and polls
`GET /tasks/{id}` once per second so you can see the queued state, worker pickup,
streamed stdout, and final HTML reply draft.

Run it locally with npm:

```bash
cd scheduler_module/frontend
npm install
npm run dev
```

Then open `https://127.0.0.1:4173`.

Notes:

- The Vite dev server uses local HTTPS via `@vitejs/plugin-basic-ssl`.
- API requests are proxied to the Rust gateway on `http://127.0.0.1:9100`.
- If `codex` is not installed locally, the worker falls back to the existing
  synthesized stub response instead of a live model stream.

For multi-tenant requests, the scheduler now partitions workspaces as:

```text
.workspace/tasks/<tenant>/<account>/<task-id>/
```

Each task gets a `workspace_manifest.json` that records the stable workspace key,
identity URI, memory URI, and credential references needed by downstream workers.

## ACI workspace tree

The production Azure Container Instances runner uses a task workspace with paths like
these during a Codex run:

```text
<workspace>/
  AGENTS.md
  SOUL.md
  incoming_email/
  incoming_attachments/
  memory/
  references/
  reply_email_draft.html
  reply_email_attachments/
  .codex/
  .codex_remote_prompt.txt
  .run_task_trace/
  .secrets/
  scheduler_snapshot.json
  thread_state.json
```

- `AGENTS.md`: operator guidance for the agent, including repo conventions.
- `SOUL.md`: persona and tone instructions.
- `incoming_email/`: merged inbound request plus raw provider payloads and thread history.
- `incoming_attachments/`: merged attachment view for the active thread.
- `memory/`: durable per-user facts that can be reused across runs.
- `references/`: prior thread artifacts or other saved reference material.
- `reply_email_draft.html`: final HTML reply that the worker should send.
- `reply_email_attachments/`: files to attach to the outgoing reply.
- `.codex/`: Codex home directory, config, logs, and local state inside the container.
- `.codex_remote_prompt.txt`: the fully assembled top-level prompt that launched Codex.
- `.run_task_trace/`: runner metadata, prompt snapshots, and execution logs for debugging.
- `.secrets/`: mounted credentials for supported CLIs and service integrations.
- `scheduler_snapshot.json`: scheduler-provided state snapshot for this run.
- `thread_state.json`: normalized thread metadata and routing state.

Some output paths are created during execution instead of being pre-populated by the
runner. `DoWhiz Core Lite` does not generate every one of those files itself. The host
runner may inject them.

The repo does guarantee a smaller per-task contract with these well-known paths under
`.workspace/tasks/<tenant>/<account>/<task-id>/`:

```text
.workspace/tasks/<tenant>/<account>/<task-id>/
  task_prompt.txt
  codex_system_prompt.md
  workspace_manifest.json
  incoming_email/
  incoming_attachments/
  reply_email_draft.html
  reply_email_attachments/
  .task_stdout.log
  .task_secrets.env
```

- `task_prompt.txt`: the combined prompt passed to `codex exec`.
- `codex_system_prompt.md`: the stable system prompt that explains the workspace
  contract, skill usage, and tool expectations for Codex.
- `workspace_manifest.json`: scheduler-owned workspace identity and routing metadata.
- `incoming_email/`: canonical inbound artifacts persisted by the gateway.
- `incoming_attachments/`: decoded inbound attachments for the active thread.
- `reply_email_draft.html`: the draft reply produced by the task runner.
- `reply_email_attachments/`: outbound attachments produced during the task.
- `.task_stdout.log`: captured stdout from the local or container execution path.
- `.task_secrets.env`: optional task-scoped secrets that the runner loads for the task.

## Containerized Codex boundary

`run_task_module` supports a local simulation path and a container path.
When `RUN_TASK_USE_CONTAINER=1`, it builds a `docker run` invocation that mounts the
workspace and delegates execution to `containers/codex-runner/entrypoint.sh`.

The actual Codex execution contract now lives in
`containers/codex-runner/exec_codex.sh`. The image supports two modes:

- `RUN_TASK_CONTAINER_MODE=one_shot`: start one container per task.
- `RUN_TASK_CONTAINER_MODE=warm_pool`: keep a long-lived container alive and execute
  `/app/exec_codex.sh` via `docker exec` after copying only the active task workspace
  into the container and scrubbing it afterward.

Build the runner image with:

```bash
docker build -t dowhiz/codex-runner:latest -f containers/codex-runner/Dockerfile .
```

The image now installs `@openai/codex` and the entrypoint writes `~/.codex/config.toml`
from the environment. Use `OPENAI_API_KEY` for the default provider, or supply the
`AZURE_OPENAI_*` variables to normalize an Azure-compatible endpoint into the same
runtime contract.

The repo-level Codex system prompt lives at
`run_task_module/prompts/codex_system_prompt.md`. `run_task_module` writes that file
into every task workspace as `codex_system_prompt.md` and prepends it to
`task_prompt.txt` so the runtime instructions stay explicit and versioned.

If a task needs per-user credentials such as a workspace SAS token, write them into
`<workspace>/.task_secrets.env` right before execution or pass specific host variables
through `RUN_TASK_CONTAINER_ENV_PASSTHROUGH`. Warm-pool mode no longer mounts the full
task tree into the container, so only the current task workspace is exposed during each
execution.

## Inbound webhooks

The gateway exposes two ingress paths:

- `POST /tasks`: submit a normalized task directly
- `POST /webhooks/postmark/inbound`: accept a Postmark inbound webhook, persist the raw
  payload under `incoming_email/`, decode any merged attachments into
  `incoming_attachments/`, and queue the task for the worker

## Outbound delivery

`send_emails_module` now supports both preview generation and actual Postmark delivery.
With `OUTBOUND_DELIVERY_MODE=postmark`, the worker will POST the finished reply to the
Postmark `/email` API and write `delivery_report.json` alongside `transport_preview.json`.

Required env:

```bash
POSTMARK_SERVER_TOKEN=...
POSTMARK_FROM=bot@example.com
```

## Notes

- This repo is meant to be read, modified, and extended quickly.
- It is not a drop-in replacement for the full upstream deployment.
- The design goal is to make the scheduler/worker/container core obvious and easy to test.
