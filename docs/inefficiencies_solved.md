# Inefficiencies Solved

Audit date: April 9, 2026
Source audited: `KnoWhiz/DoWhiz` on branch `dev`

## What I found in the source repo

The upstream repo is impressive, but the core Rust service has accumulated enough
surface area that everyday development is now paying a tax in a few places.

### 1. Critical logic is concentrated in very large files

From the audited tree:

- `scheduler_module/src/service/auth.rs`: 7,067 lines
- `run_task_module/src/run_task/codex.rs`: 5,695 lines
- `scheduler_module/src/service/chat_history.rs`: 3,144 lines
- `scheduler_module/src/bin/inbound_gateway/handlers.rs`: 2,624 lines
- `scheduler_module/src/scheduler/actions.rs`: 2,487 lines

Large files are not just a style issue. They make code ownership fuzzy, increase merge
conflicts, and slow down targeted testing because one file tends to carry many unrelated
responsibilities.

### 2. One crate owns too many product domains

The audited `scheduler_module` combines:

- ingress and webhook handling
- OAuth and auth routes
- analytics
- billing
- Google Workspace polling
- Discord gateway handling
- Notion browser integration
- grocery tooling
- identity lookup and multiple CLIs
- scheduler state and outbound routing

That breadth makes the scheduler crate hard to reason about as a single unit, even
though the core product path is conceptually simple.

### 3. The execution boundary is buried inside a large mixed-purpose runner

`run_task_module/src/run_task/codex.rs` mixes:

- workspace preparation
- environment shaping
- Docker execution
- Azure ACI execution
- Browserbase handling
- GitHub auth handling
- human approval gate wiring
- reply artifact validation
- cleanup and retry logic

Those are related concerns, but not one concern. The core execution contract gets harder
to extract because infrastructure details and policy details live in the same place.

### 4. The service repo mixes core runtime with a lot of adjacent material

At the repository root, the audited tree includes:

- the Rust backend
- the website frontend
- large documentation folders
- external dependencies and submodules
- PDFs and binary artifacts
- multiple Dockerfiles and operational assets

That is convenient for one repository, but it makes it harder to isolate and evolve the
backend runtime as a standalone system.

### 5. Testing skews heavily toward live end-to-end coverage

The service workspace contains 27 Rust test files, with many integration tests named as
live or end-to-end scenarios. Those are valuable, but they also raise the cost of quick
local iteration when the architectural seams are not small and clean.

## What this lightweight replica changes

## 1. It narrows the repo to the essential execution path

This repo keeps only:

- an inbound gateway
- a queue
- a worker loop
- a task runner
- an outbound reply preview

That makes the main product path visible in minutes instead of hours.

## 2. It separates responsibilities by runtime boundary

In this repo:

- `scheduler_module` owns ingress, workspace creation, queueing, and worker orchestration
- `run_task_module` owns task execution and the container boundary
- `send_emails_module` owns outbound preview assembly

This is a smaller and more stable set of seams than the source repo currently exposes.

## 3. It turns containerized Codex execution into an explicit contract

Instead of embedding container concerns inside a large mixed runner file, this repo uses:

- a small container runner module
- a sample Dockerfile
- a single entrypoint script contract

That makes the execution backend replaceable. Local simulation, Docker, or a future
remote backend can all conform to the same interface.

## 4. It uses a local file queue to make the scheduler easy to inspect

The upstream repo supports production-grade queue and storage backends. For a lightweight
replica, those dependencies are noise. A file queue makes state transitions obvious:

- `pending`
- `claimed`
- `completed`
- `failed`

That is enough to explain the worker lifecycle without pulling in cloud infra.

## 5. It optimizes for modularity over feature breadth

This repo intentionally leaves out:

- OAuth routing
- billing
- analytics
- multi-channel adapters
- legacy workspace product layers
- large live E2E harnesses

Those are important in production, but they should be layered on top of the core runtime,
not embedded into the smallest reproducible architecture.

## Result

The result is a repo that is easier to:

- review
- onboard into
- test locally
- replace pieces inside
- document accurately

It is not feature-complete relative to `KnoWhiz/DoWhiz`. It is a cleaner substrate for
the essential scheduler/worker/container flow.
