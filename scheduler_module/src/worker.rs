use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use run_task_module::{run_task, RunTaskParams};
use send_emails_module::{
    build_outbound_preview, send_via_postmark, write_delivery_report, write_preview_json,
    OutboundMessage, PostmarkConfig,
};
use uuid::Uuid;

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
        let Some(mut envelope) = self.queue.claim_next(&self.worker_id)? else {
            return Ok(false);
        };

        let workspace_dir = PathBuf::from(&envelope.task.workspace_dir);
        match self.process_workspace(&workspace_dir, &envelope.task.request.prompt) {
            Ok(()) => {
                self.queue.complete(envelope)?;
            }
            Err(err) => {
                self.queue.fail(envelope.clone(), err.to_string())?;
            }
        }
        Ok(true)
    }

    fn process_workspace(&self, workspace_dir: &PathBuf, prompt: &str) -> Result<()> {
        fs::create_dir_all(&self.config.tasks_root)?;
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

        let subject = format!("Re: {}", envelope_subject(&workspace_dir)?);
        let preview = build_outbound_preview(
            &output.reply_html_path,
            &output.reply_attachments_dir,
            subject.clone(),
        )?;
        write_preview_json(workspace_dir.join("transport_preview.json"), &preview)?;
        self.deliver_reply(workspace_dir, &preview, subject)?;
        Ok(())
    }

    fn deliver_reply(
        &self,
        workspace_dir: &PathBuf,
        preview: &send_emails_module::OutboundPreview,
        subject: String,
    ) -> Result<()> {
        if self.config.outbound_mode != OutboundMode::Postmark {
            return Ok(());
        }

        let request = read_task_request(workspace_dir)?;
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
                reply_to: Some(request.customer_email),
                tag: self.config.postmark_tag.clone(),
            },
            &workspace_dir.join("reply_email_attachments"),
        )?;

        write_delivery_report(workspace_dir.join("delivery_report.json"), &report)?;
        Ok(())
    }
}

fn read_task_request(workspace_dir: &PathBuf) -> Result<crate::models::InboundTaskRequest> {
    let payload = fs::read_to_string(workspace_dir.join("task_request.json"))?;
    Ok(serde_json::from_str(&payload)?)
}

fn envelope_subject(workspace_dir: &PathBuf) -> Result<String> {
    Ok(read_task_request(workspace_dir)?.subject)
}
