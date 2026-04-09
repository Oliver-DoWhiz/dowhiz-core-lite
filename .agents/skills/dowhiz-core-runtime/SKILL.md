---
name: dowhiz-core-runtime
description: Use when working on dowhiz-core-lite runtime paths so changes stay aligned with the repo's scheduler, worker, and container boundaries.
---

# DoWhiz Core Runtime

Use this skill before changing the execution path.

## Read first

- `README.md`
- `TECHNICAL_DEBT.md`
- `docs/scaling_and_runtime.md`

## Runtime boundaries

- `scheduler_module/` owns ingress, queueing, workspace initialization, and worker orchestration.
- `run_task_module/` owns workspace preparation plus local/container execution.
- `send_emails_module/` owns outbound preview and provider delivery.

Do not collapse these boundaries by moving mail transport, queue orchestration, and task execution into one module.

## Workspace contract

- Per-task runtime state lives under `.workspace/tasks/<tenant>/<account>/<task-id>/`.
- `workspace_manifest.json` is the source of truth for the stable workspace key.
- `incoming_email/`, `incoming_attachments/`, and `reply_email_attachments/` are part of the task contract.

## Container contract

- `one_shot` mode may mount the current task workspace directly.
- `warm_pool` mode must only expose the active task workspace and must scrub it from the long-lived container after execution.
- Put task-scoped secrets in `.task_secrets.env` instead of broad process-global config.
