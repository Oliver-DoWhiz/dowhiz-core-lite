---
name: dowhiz-inbound-email
description: Use when adding or modifying inbound email/webhook handling so provider payloads are normalized into task workspaces without breaking the scheduler boundary.
---

# DoWhiz Inbound Email

Use this skill when touching Postmark inbound handling or email-derived task creation.

## Required behavior

- Accept provider payloads at the gateway edge.
- Translate the provider payload into `InboundTaskRequest`.
- Persist the raw inbound payload under `incoming_email/`.
- Decode merged attachments into `incoming_attachments/`.
- Queue the task only after the inbound artifacts are written.

## Keep the adapter thin

- Provider-specific parsing belongs in a focused module such as `inbound_email.rs`.
- The scheduler should still see a normal `InboundTaskRequest`.
- The worker should not need to understand Postmark-specific fields to run the task.

## Reply expectations

- Preserve the sender address as the default reply target.
- Prefer stripped reply text when present, but keep the raw payload for auditability.
