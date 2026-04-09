# Scaling And Runtime Notes

This document answers the follow-up architecture questions from issue #2 and ties them
back to concrete changes in the repo.

## 1. Where the Codex logic lives

The Codex execution contract now lives in:

- `containers/codex-runner/exec_codex.sh`
- `containers/codex-runner/entrypoint.sh`
- `run_task_module/src/container.rs`

`exec_codex.sh` is the script that a real container would use to run `codex exec` inside
the mounted task workspace. The Rust side only prepares the workspace, injects the task
metadata, and chooses whether to run a one-shot container or a warm pooled container.

That keeps the boundary clean:

```text
scheduler -> workspace manifest -> container runtime -> exec_codex.sh -> codex
```

## 2. Managing N users and N workspaces

The scheduler no longer assumes a single flat task directory. It now derives a stable
workspace key and path:

```text
<tenant>/<account>/<task-id>
```

The request model now supports:

- `tenant_id`
- `account_id`
- `memory_uri`
- `identity_uri`
- `credential_refs`

The scheduler writes those into `workspace_manifest.json`. That means a worker does not
need to guess which workspace to load. The queue item already points to a concrete
workspace path, and the manifest keeps the stable logical key even if the physical path
changes later.

For a true multi-node deployment, I would keep the same manifest shape but store the
manifest in object storage or a database-backed workspace registry so any worker can
resolve the same workspace key.

## 3. Docker image build and warm container strategy

The Dockerfile already existed at `containers/codex-runner/Dockerfile`, but the runtime
contract was too vague. The repo now supports two execution modes:

- `one_shot`: one `docker run --rm` per task
- `warm_pool`: one long-lived container per mounted task root, then `docker exec` for
  each task

Warm pool behavior is implemented in `run_task_module/src/container.rs`. The pool is
useful because:

- the image is pulled and started once
- toolchains stay hot
- repeated agent tasks avoid per-task container bootstrap cost

To get task data into the container without cloud infrastructure:

1. Mount the shared task root into the container.
2. Resolve each task to a relative workspace path.
3. Pass task-specific files and env vars into `docker exec`.
4. Put per-task secrets into `.task_secrets.env` right before execution, then delete it
   after the task if desired.

That works on a single VM, a bare-metal box, or any host with Docker access. Cloud
services are helpful, but not required for the basic orchestration model.

## 4. Full outbound Postmark implementation

`send_emails_module` now supports:

- preview assembly
- attachment enumeration
- attachment base64 encoding
- POSTing to Postmark `/email`
- writing a structured `delivery_report.json`

The worker keeps `transport_preview.json` for local inspection and optionally sends the
real email when `OUTBOUND_DELIVERY_MODE=postmark`.

## 5. Multi-user memory and identity storage

The manifest fields are intentionally URIs instead of provider-specific paths. That keeps
the runtime open to several storage strategies:

- current DoWhiz approach: Supabase/Postgres for identities plus Azure Blob for memory
- Postgres only: store identity rows and memo markdown blobs in the same database
- object store + metadata DB: S3/R2/Blob for files, Postgres or SQLite/LiteFS for the
  index
- local-first single-node deployment: SQLite for identities and a versioned filesystem
  tree for memories

If I wanted a simple non-cloud deployment, I would use:

- Postgres or SQLite for identity/account metadata
- MinIO or plain disk for memory snapshots
- workspace manifests with immutable object keys

That gives predictable lookups and avoids binding the worker contract to one vendor.

## 6. Multiple inbound gateways and distributed coordination

The current file queue is intentionally local and good for a single-node replica. For a
distributed deployment, I would split the system like this:

```text
HTTPS ingress / load balancer
  -> stateless inbound gateways
  -> durable queue
  -> worker pool
  -> workspace registry + memory store + identity store
```

A few reasonable coordination options:

- Postgres queue table with `FOR UPDATE SKIP LOCKED`
- Redis streams with consumer groups
- SQS or NATS JetStream

The key rule is that queue ownership and workspace ownership must be decoupled. Gateways
should only validate requests and enqueue metadata. Workers should resolve workspace
state from the registry and then run the task.

## 7. Why the gateway uses a TCP listener

`service.rs` does not process raw TCP payloads. It binds a TCP socket because HTTP servers
need a socket transport. Axum then serves HTTP JSON on top of that listener.

That means:

- TCP is the transport
- HTTP is the application protocol
- HTTPS should terminate at the proxy/load balancer layer

For production, I would keep the Rust service speaking plain HTTP behind a reverse proxy
unless there is a specific need for end-to-end TLS inside the private network.
