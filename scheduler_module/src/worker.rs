use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use run_task_module::{run_task, RunTaskParams};
use send_emails_module::{
    build_outbound_preview, send_via_postmark, write_delivery_report, write_preview_json,
    OutboundMessage, PostmarkConfig,
};
use uuid::Uuid;

use crate::account_registry::persist_workspace_memory;
use crate::config::{OutboundMode, WorkerConfig};
use crate::queue::FileQueue;

#[derive(Debug, Clone)]
pub struct WorkerService {
    queue: FileQueue,
    config: WorkerConfig,
    worker_id: String,
}

impl WorkerService {
    pub fn new(config: WorkerConfig) -> Result<Self> {
        Ok(Self {
            queue: FileQueue::new(config.queue_root.clone())?,
            config,
            worker_id: format!("worker-{}", Uuid::new_v4()),
        })
    }

    pub async fn run_forever(&self) -> Result<()> {
        loop {
            if !self.process_once().await? {
                tokio::time::sleep(self.config.poll_interval).await;
            }
        }
    }

    pub async fn process_once(&self) -> Result<bool> {
        let Some(envelope) = self.queue.claim_next(&self.worker_id)? else {
            tracing::debug!(
                worker_id = %self.worker_id,
                poll_interval_ms = self.config.poll_interval.as_millis(),
                "no queued task available for worker poll"
            );
            return Ok(false);
        };

        let workspace_dir = PathBuf::from(&envelope.task.workspace_dir);
        tracing::info!(
            task_id = %envelope.task.id,
            worker_id = %self.worker_id,
            workspace_dir = %workspace_dir.display(),
            execution_mode = %execution_mode_label(self.config.use_container, self.config.container_mode),
            "starting worker task processing"
        );
        match self.process_workspace(
            &envelope.task.id,
            &envelope.task.workspace_key,
            &workspace_dir,
            &envelope.task.request.prompt,
        ) {
            Ok(()) => {
                tracing::info!(
                    task_id = %envelope.task.id,
                    worker_id = %self.worker_id,
                    "worker task processing finished successfully"
                );
                self.queue.complete(envelope)?;
            }
            Err(err) => {
                tracing::error!(
                    task_id = %envelope.task.id,
                    worker_id = %self.worker_id,
                    error = %err,
                    "worker task processing failed"
                );
                self.queue.fail(envelope.clone(), err.to_string())?;
            }
        }
        Ok(true)
    }

    fn process_workspace(
        &self,
        task_id: &str,
        workspace_key: &str,
        workspace_dir: &PathBuf,
        prompt: &str,
    ) -> Result<()> {
        fs::create_dir_all(&self.config.tasks_root)?;
        tracing::info!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            prompt_chars = prompt.chars().count(),
            use_container = self.config.use_container,
            container_mode = %container_mode_label(self.config.container_mode),
            "dispatching task to run_task_module"
        );
        let output = run_task(&RunTaskParams {
            workspace_dir: workspace_dir.clone(),
            prompt: prompt.to_string(),
            use_container: self.config.use_container,
            container_image: self.config.container_image.clone(),
            container_mode: self.config.container_mode,
            container_workspace_root: self.config.container_workspace_root.clone(),
            container_pool_name: self.config.container_pool_name.clone(),
            env_passthrough: self.config.container_env_passthrough.clone(),
        })?;
        tracing::info!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            stdout_bytes = output.stdout.len(),
            reply_html_path = %output.reply_html_path.display(),
            attachments_dir = %output.reply_attachments_dir.display(),
            "task runner returned output"
        );
        let request = read_task_request(workspace_dir)?;
        persist_workspace_memory(workspace_dir, &request.memory_uri)?;
        tracing::debug!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            memory_uri = %request.memory_uri,
            "persisted workspace memory back to account storage"
        );

        let subject = format!("Re: {}", request.subject);
        let preview = build_outbound_preview(
            &output.reply_html_path,
            &output.reply_attachments_dir,
            subject.clone(),
        )?;
        tracing::info!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            subject = %subject,
            attachment_count = preview.attachment_names.len(),
            "built outbound preview"
        );
        write_preview_json(workspace_dir.join("transport_preview.json"), &preview)?;
        tracing::debug!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            preview_path = %workspace_dir.join("transport_preview.json").display(),
            "wrote outbound preview artifact"
        );
        self.deliver_reply(task_id, workspace_key, workspace_dir, &request, &preview, subject)?;
        Ok(())
    }

    fn deliver_reply(
        &self,
        task_id: &str,
        workspace_key: &str,
        workspace_dir: &PathBuf,
        request: &crate::models::InboundTaskRequest,
        preview: &send_emails_module::OutboundPreview,
        subject: String,
    ) -> Result<()> {
        if self.config.outbound_mode != OutboundMode::Postmark {
            tracing::info!(
                task_id = task_id,
                workspace_key = workspace_key,
                workspace_dir = %workspace_dir.display(),
                outbound_mode = "preview_only",
                "skipping outbound send and leaving preview artifacts in workspace"
            );
            return Ok(());
        }

        let from = self
            .config
            .postmark_from
            .clone()
            .context("POSTMARK_FROM is required when OUTBOUND_DELIVERY_MODE=postmark")?;
        let server_token = self
            .config
            .postmark_server_token
            .clone()
            .context("POSTMARK_SERVER_TOKEN is required when OUTBOUND_DELIVERY_MODE=postmark")?;
        let to = if request.reply_to.trim().is_empty() {
            request.customer_email.clone()
        } else {
            request.reply_to.clone()
        };
        tracing::info!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            to = %to,
            subject = %subject,
            attachment_count = preview.attachment_names.len(),
            message_stream = ?self.config.postmark_message_stream,
            "sending outbound reply via Postmark"
        );

        let report = send_via_postmark(
            &PostmarkConfig {
                api_base_url: self.config.postmark_api_base_url.clone(),
                server_token,
                message_stream: self.config.postmark_message_stream.clone(),
            },
            &OutboundMessage {
                from,
                to,
                subject,
                html_body: preview.html_body.clone(),
                reply_to: Some(request.customer_email.clone()),
                tag: self.config.postmark_tag.clone(),
            },
            &workspace_dir.join("reply_email_attachments"),
        )?;

        write_delivery_report(workspace_dir.join("delivery_report.json"), &report)?;
        tracing::info!(
            task_id = task_id,
            workspace_key = workspace_key,
            workspace_dir = %workspace_dir.display(),
            to = %report.to,
            subject = %report.subject,
            message_id = ?report.message_id,
            "persisted outbound delivery report"
        );
        Ok(())
    }
}

fn read_task_request(workspace_dir: &PathBuf) -> Result<crate::models::InboundTaskRequest> {
    let payload = fs::read_to_string(workspace_dir.join("task_request.json"))?;
    Ok(serde_json::from_str(&payload)?)
}

fn execution_mode_label(use_container: bool, container_mode: run_task_module::ContainerMode) -> &'static str {
    if !use_container {
        "local"
    } else {
        match container_mode {
            run_task_module::ContainerMode::OneShot => "container_one_shot",
            run_task_module::ContainerMode::WarmPool => "container_warm_pool",
        }
    }
}

fn container_mode_label(container_mode: run_task_module::ContainerMode) -> &'static str {
    match container_mode {
        run_task_module::ContainerMode::OneShot => "one_shot",
        run_task_module::ContainerMode::WarmPool => "warm_pool",
    }
}
