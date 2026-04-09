use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use crate::models::{InboundTaskRequest, QueuedTask, TaskStatus};
use crate::queue::FileQueue;
use crate::workspace_registry::{initialize_workspace, plan_workspace};

#[derive(Debug, Clone)]
pub struct TaskScheduler {
    queue: FileQueue,
    tasks_root: PathBuf,
}

impl TaskScheduler {
    pub fn new(queue: FileQueue, tasks_root: impl Into<PathBuf>) -> Result<Self> {
        let tasks_root = tasks_root.into();
        std::fs::create_dir_all(&tasks_root)?;
        Ok(Self { queue, tasks_root })
    }

    pub fn submit(&self, request: InboundTaskRequest) -> Result<QueuedTask> {
        let task_id = Uuid::new_v4().to_string();
        let created_at = Utc::now();
        let layout = plan_workspace(&self.tasks_root, &task_id, &request);
        let manifest = initialize_workspace(&layout, &task_id, created_at, &request)?;

        let task = QueuedTask {
            id: task_id,
            created_at,
            status: TaskStatus::Pending,
            request,
            workspace_key: manifest.workspace_key,
            workspace_dir: manifest.workspace_dir,
        };

        self.queue.enqueue(&task)?;
        Ok(task)
    }
}
