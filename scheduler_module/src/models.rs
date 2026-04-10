use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateTaskRequest {
    pub customer_email: String,
    pub subject: String,
    pub prompt: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub reply_to: String,
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub attachment_refs: Vec<AttachmentUploadRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentUploadRef {
    pub upload_id: String,
    pub file_name: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadAttachmentsResponse {
    pub attachments: Vec<AttachmentUploadRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundTaskRequest {
    pub customer_email: String,
    pub subject: String,
    pub prompt: String,
    #[serde(default = "default_channel")]
    pub channel: String,
    #[serde(default)]
    pub reply_to: String,
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default)]
    pub account_id: String,
    #[serde(default)]
    pub memory_uri: String,
    #[serde(default)]
    pub identity_uri: String,
    #[serde(default)]
    pub credential_refs: Vec<String>,
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
    pub workspace_key: String,
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
