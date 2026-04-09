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
cargo run -p scheduler_module --bin rust_service
```

In another terminal:

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

The worker writes per-task artifacts under `.workspace/tasks/<task-id>/`.

## Containerized Codex boundary

`run_task_module` supports a local simulation path and a container path.
When `RUN_TASK_USE_CONTAINER=1`, it builds a `docker run` invocation that mounts the
workspace and delegates execution to `containers/codex-runner/entrypoint.sh`.

The sample image is intentionally minimal. Replace its internals with a real Codex CLI
install in environments where the agent runtime is available.

## Notes

- This repo is meant to be read, modified, and extended quickly.
- It is not a drop-in replacement for the full upstream deployment.
- The design goal is to make the scheduler/worker/container core obvious and easy to test.
