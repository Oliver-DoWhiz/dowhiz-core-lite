use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundTaskRequest {
    pub customer_email: String,
    pub subject: String,
    pub prompt: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub reply_to: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedTask {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub status: TaskStatus,
    pub request: InboundTaskRequest,
    pub workspace_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEnvelope {
    pub task: QueuedTask,
    pub worker_id: Option<String>,
    pub error: Option<String>,
}

fn default_channel() -> String {
    "email".to_string()
}
