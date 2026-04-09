use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::models::{InboundTaskRequest, QueuedTask, TaskStatus};
use crate::queue::FileQueue;

#[derive(Debug, Clone)]
pub struct TaskScheduler {
    queue: FileQueue,
    tasks_root: PathBuf,
}

impl TaskScheduler {
    pub fn new(queue: FileQueue, tasks_root: impl Into<PathBuf>) -> Result<Self> {
        let tasks_root = tasks_root.into();
        fs::create_dir_all(&tasks_root)?;
        Ok(Self { queue, tasks_root })
    }

    pub fn submit(&self, request: InboundTaskRequest) -> Result<QueuedTask> {
        let task_id = Uuid::new_v4().to_string();
        let workspace_dir = self.tasks_root.join(&task_id);
        self.prepare_workspace(&workspace_dir, &request)?;

        let task = QueuedTask {
            id: task_id,
            created_at: Utc::now(),
            status: TaskStatus::Pending,
            request,
            workspace_dir: workspace_dir.display().to_string(),
        };

        self.queue.enqueue(&task)?;
        Ok(task)
    }

    fn prepare_workspace(&self, workspace_dir: &PathBuf, request: &InboundTaskRequest) -> Result<()> {
        fs::create_dir_all(workspace_dir.join("incoming_email"))?;
        fs::create_dir_all(workspace_dir.join("incoming_attachments"))?;
        fs::create_dir_all(workspace_dir.join("reply_email_attachments"))?;
        fs::write(
            workspace_dir.join("incoming_email/thread_request.md"),
            render_thread_request(request),
        )?;
        fs::write(
            workspace_dir.join("task_request.json"),
            serde_json::to_string_pretty(request)?,
        )?;
        Ok(())
    }
}

fn render_thread_request(request: &InboundTaskRequest) -> String {
    format!(
        "# Incoming request\n\nFrom: {}\nSubject: {}\nChannel: {}\nReply-To: {}\n\n## Prompt\n{}\n",
        request.customer_email,
        request.subject,
        request.channel,
        request.reply_to,
        request.prompt
    )
}
