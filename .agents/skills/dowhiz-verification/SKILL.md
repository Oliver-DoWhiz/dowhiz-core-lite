---
name: dowhiz-verification
description: Use when validating changes in dowhiz-core-lite so Codex does not claim delivery, webhook support, or runtime behavior that the codebase does not actually implement.
---

# DoWhiz Verification

Before saying a feature exists, verify the implementation surface directly.

## Delivery claims

- Do not claim outbound delivery unless `send_emails_module` writes the preview and, when configured, `delivery_report.json`.
- Do not claim inbound support unless there is an actual HTTP route plus artifact persistence in the task workspace.

## Runtime claims

- Check `run_task_module` for the real container and local execution behavior.
- Check `.env.example` for the documented runtime knobs.
- Check the Dockerfile and entrypoint scripts before claiming the image installs or configures Codex.

## Verification checklist

- Read the touched module and its adjacent contract file.
- Add or update unit tests for parsing or path-planning logic when practical.
- Run `cargo test` when the toolchain is available.
- If the toolchain is unavailable, state that explicitly instead of implying the tests passed.
