# Scaling And Runtime Notes

This document answers the follow-up architecture questions from issue #2 and issue #14
and ties them back to concrete changes in the repo.

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
contract was too vague. The image now installs `@openai/codex`, and the entrypoint
normalizes `OPENAI_API_KEY` / `OPENAI_BASE_URL` or `AZURE_OPENAI_*` variables into the
runtime config that `codex exec` reads. The repo supports two execution modes:

- `one_shot`: one `docker run --rm` per task
- `warm_pool`: one long-lived container with no tenant workspace mounted, then
  `docker cp` + `docker exec` + `docker cp` for each task

Warm pool behavior is implemented in `run_task_module/src/container.rs`. The pool is
useful because:

- the image is pulled and started once
- toolchains stay hot
- repeated agent tasks avoid per-task container bootstrap cost

To get task data into the container without exposing the entire file share:

1. Keep the warm pool container provisioned from the deploy-time image only.
2. Resolve each task to a relative workspace key such as `tenant/account/task-id`.
3. Copy just that workspace into the running container before `docker exec`.
4. Copy the finished workspace back out and remove the container-side copy after the run.
5. Put per-task secrets into `.task_secrets.env` right before execution, then delete it
   after the task if desired.

That works on a single VM, a bare-metal box, or any host with Docker access. It also
maps cleanly onto a queue-driven deployment because the warm container no longer depends
on a broad bind mount existing ahead of time.

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

The current code still allows `InboundTaskRequest` to carry `memory_uri`,
`identity_uri`, and `credential_refs` directly. That is acceptable for an internal
normalized request, but I would not keep that as the public browser-facing contract.

The cleaner boundary is:

1. Public client sends the user-visible task fields such as `customer_email`, `subject`,
   `prompt`, channel metadata, and optionally a stable `account_id`.
2. Gateway resolves `account_id` if it was not provided, for example by looking up
   `customer_email`.
3. Gateway looks up `memory_uri`, `identity_uri`, and `credential_refs` from a
   server-side account registry.
4. Gateway enqueues an enriched internal `InboundTaskRequest`.

If DoWhiz wants the smallest possible first step, a simple KV layer is enough:

- `customer_email -> account_id`
- `account_id -> memory_uri`
- `account_id -> identity_uri`

That keeps memory lookup on the server side where it belongs, while still matching the
current workspace manifest shape.

## 6. Attachment handling for direct tasks and the frontend

Codex does not need a special attachment API once the task starts. It already knows how
to work with normal files in the workspace. The important part is what the gateway does
before the task is queued.

The email ingress path already shows the right model:

- raw provider payload goes under `incoming_email/`
- decoded files go under `incoming_attachments/`
- a small `thread_manifest.json` records attachment names
- the task prompt and workspace prompt tell Codex where to look

I would reuse that same contract for browser-submitted tasks. The browser should not
base64-embed large files into the JSON body of `POST /tasks`. That creates slow uploads,
large request bodies, and unnecessary duplication.

Two reasonable options:

- Local/dev-only: submit `multipart/form-data` directly to the gateway so the proof of
  concept can support drag-and-drop without new storage infrastructure.
- Production/scalable: upload files first, then send only attachment references in the
  final task creation request.

For the scalable path, the public request should look more like this:

```json
{
  "customer_email": "dtang04@uchicago.edu",
  "account_id": "user_42",
  "subject": "Review these files",
  "prompt": "Use the attached spreadsheet and PDF.",
  "channel": "email",
  "reply_to": "dtang04@uchicago.edu",
  "attachment_refs": [
    {
      "file_name": "model.xlsx",
      "content_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
      "size_bytes": 182044,
      "storage_uri": "az://task-uploads/user_42/abc123"
    },
    {
      "file_name": "notes.pdf",
      "content_type": "application/pdf",
      "size_bytes": 90211,
      "storage_uri": "az://task-uploads/user_42/def456"
    }
  ]
}
```

Then the gateway can:

1. Resolve the account and memory metadata.
2. Download or copy each referenced blob into `incoming_attachments/`.
3. Write the attachment manifest alongside the files.
4. Queue the same normalized worker task shape used by email ingress.

If DoWhiz wants to avoid storage URIs in the client request, the same pattern can use
gateway-issued `upload_id` values instead. That is now the implemented local POC path:
`POST /uploads` stages the raw bytes under a local upload root, returns `upload_id`
refs, and `POST /tasks` carries only those refs. The gateway then copies the staged
files into `incoming_attachments/` before queueing the task. The important design
choice is the same: `POST /tasks` should carry attachment metadata or references, not
the file bytes themselves.

On the frontend side, that means the drag-and-drop area should upload files immediately,
show the user a pending attachment list, and submit the final task with only those
returned refs.

## 7. Multiple inbound gateways and distributed coordination

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

If DoWhiz keeps using an Azure Storage Queue plus a workspace SAS token, I would keep the
queue message minimal:

- workspace key
- storage URI or SAS-scoped fetch URI for the task workspace
- identity and memory references
- requested channel metadata

The worker can then fetch only that task workspace into its local staging area, run
`docker cp` into the already-provisioned warm container, execute the task, and upload the
resulting reply artifacts back to durable storage. That preserves the current queue-based
shape without exposing a tenant-wide share inside the container.

## 8. Inbound webhooks

Outbound delivery is no longer the only mail integration. The gateway now also accepts
`POST /webhooks/postmark/inbound`, persists the raw provider payload, decodes merged
attachments into the task workspace, and then queues a normal `InboundTaskRequest`.

That keeps the scheduler boundary clean:

```text
postmark webhook -> inbound_email adapter -> task scheduler -> queue -> worker
```

## 9. Why the gateway uses a TCP listener

`service.rs` does not process raw TCP payloads. It binds a TCP socket because HTTP servers
need a socket transport. Axum then serves HTTP JSON on top of that listener.

That means:

- TCP is the transport
- HTTP is the application protocol
- HTTPS should terminate at the proxy/load balancer layer

For production, I would keep the Rust service speaking plain HTTP behind a reverse proxy
unless there is a specific need for end-to-end TLS inside the private network.
