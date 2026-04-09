use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use run_task_module::{run_task, RunTaskParams};
use send_emails_module::{build_outbound_preview, write_preview_json};
use uuid::Uuid;

use crate::config::WorkerConfig;
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
        })?;

        let preview = build_outbound_preview(
            &output.reply_html_path,
            &output.reply_attachments_dir,
            "DoWhiz task result".to_string(),
        )?;
        write_preview_json(workspace_dir.join("transport_preview.json"), &preview)?;
        Ok(())
    }
}
