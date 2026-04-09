# Technical Debt Found And How This Repo Addresses It

Audit date: April 9, 2026
Source audited: `KnoWhiz/DoWhiz` on branch `dev`

This lightweight repo exists because the upstream codebase contains a strong core
architecture, but that core is harder to study and evolve than it should be.
The main technical debt I found was not one bug. It was the accumulation of too
many responsibilities in too few places.

## Technical debt found

### 1. Large files hide multiple responsibilities

Several upstream files are large enough that they blend unrelated concerns into a
single edit surface. Examples from the audit:

- `scheduler_module/src/service/auth.rs`: 7,067 lines
- `run_task_module/src/run_task/codex.rs`: 5,695 lines
- `scheduler_module/src/service/chat_history.rs`: 3,144 lines
- `scheduler_module/src/bin/inbound_gateway/handlers.rs`: 2,624 lines
- `scheduler_module/src/scheduler/actions.rs`: 2,487 lines

That creates review drag, weaker ownership boundaries, and more merge conflicts.

### 2. The scheduler layer owns too many product domains

In the upstream repo, the scheduler area is not only handling inbound work and
task orchestration. It also carries auth, analytics, billing, Google Workspace
polling, Discord handling, Notion browser flows, grocery tooling, and other
product-specific integrations.

That makes it difficult to isolate the critical runtime path:

`inbound request -> queue -> worker -> task runner -> reply`

### 3. The task runner mixes contract and infrastructure details

The upstream runner path combines workspace setup, Docker execution, cloud
execution, auth wiring, browser handling, human approval gate logic, output
validation, and cleanup in the same area.

Those concerns are related, but they should not all define the same boundary.

### 4. The repository mixes the core backend with adjacent surfaces

The full upstream repository contains the backend runtime together with the
website, large docs trees, operational assets, and other supporting material.

That is practical for a production monorepo, but it makes the core execution
model harder to extract and reason about on its own.

## How `dowhiz-core-lite` addresses that debt

### 1. It narrows the repo to the essential execution path

This repo keeps only the smallest useful backbone:

- `scheduler_module`: ingress, queueing, task scheduling, worker orchestration
- `run_task_module`: workspace preparation plus local/container task execution
- `send_emails_module`: outbound reply preview generation

That makes the core runtime understandable quickly.

### 2. It restores explicit module boundaries

The crates in this repo are separated by runtime responsibility instead of by
historical product growth. The result is a simpler contract between ingress,
execution, and reply generation.

### 3. It makes the execution backend replaceable

`run_task_module` exposes a small task-running interface and keeps the container
path in focused modules such as `container.rs`, `local.rs`, `types.rs`, and
`workspace.rs`.

That makes it easier to swap between local execution and container execution
without dragging every infrastructure concern through the same file.

### 4. It uses a simple queue that is easy to inspect

The file-backed queue is intentionally modest. That is a feature here, not a
limitation. It makes worker state visible and understandable without needing the
full production storage and queue stack.

### 5. It optimizes for readability and extension

This repo is not trying to be production-complete. It is trying to be the
smallest maintainable version of the scheduler/worker/container architecture,
so engineers can audit it, test it, and extend it without pulling unrelated
systems into every change.

## Related detail

For the longer audit version with the original examples and rationale, see
[`docs/inefficiencies_solved.md`](docs/inefficiencies_solved.md).
