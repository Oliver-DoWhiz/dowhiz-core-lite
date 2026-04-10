use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::models::{TaskEnvelope, TaskStatus};

#[derive(Debug, Clone)]
pub struct TaskInspector {
    queue_root: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSnapshot {
    pub id: String,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub subject: String,
    pub customer_email: String,
    pub reply_to: String,
    pub workspace_key: String,
    pub worker_id: Option<String>,
    pub error: Option<String>,
    pub stdout: String,
    pub reply_html: Option<String>,
    pub attachment_names: Vec<String>,
}

impl TaskInspector {
    pub fn new(queue_root: impl Into<PathBuf>) -> Self {
        Self {
            queue_root: queue_root.into(),
        }
    }

    pub fn get(&self, task_id: &str) -> Result<Option<TaskSnapshot>> {
        let Some(envelope) = self.find_envelope(task_id)? else {
            return Ok(None);
        };

        Ok(Some(TaskSnapshot::from_envelope(envelope)?))
    }

    fn find_envelope(&self, task_id: &str) -> Result<Option<TaskEnvelope>> {
        for dir in ["pending", "claimed", "completed", "failed"] {
            let path = self.queue_root.join(dir).join(format!("{}.json", task_id));
            if path.exists() {
                return read_envelope(&path).map(Some);
            }
        }

        Ok(None)
    }
}

impl TaskSnapshot {
    fn from_envelope(envelope: TaskEnvelope) -> Result<Self> {
        let workspace_dir = PathBuf::from(&envelope.task.workspace_dir);
        Ok(Self {
            id: envelope.task.id,
            status: envelope.task.status,
            created_at: envelope.task.created_at,
            subject: envelope.task.request.subject,
            customer_email: envelope.task.request.customer_email,
            reply_to: envelope.task.request.reply_to,
            workspace_key: envelope.task.workspace_key,
            worker_id: envelope.worker_id,
            error: envelope.error,
            stdout: read_optional_string(&workspace_dir.join(".task_stdout.log"))?.unwrap_or_default(),
            reply_html: read_optional_string(&workspace_dir.join("reply_email_draft.html"))?,
            attachment_names: list_attachment_names(&workspace_dir.join("reply_email_attachments"))?,
        })
    }
}

fn read_envelope(path: &Path) -> Result<TaskEnvelope> {
    let payload = fs::read_to_string(path)
        .with_context(|| format!("failed to read task envelope {}", path.display()))?;
    serde_json::from_str(&payload).context("failed to parse task envelope")
}

fn read_optional_string(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(fs::read_to_string(path)?))
}

fn list_attachment_names(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut names = fs::read_dir(path)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::models::{InboundTaskRequest, QueuedTask};

    #[test]
    fn returns_workspace_artifacts_for_completed_tasks() {
        let root = temp_dir("task-inspector");
        let queue_root = root.join("queue");
        let workspace_dir = root.join("tasks").join("demo-task");

        fs::create_dir_all(queue_root.join("completed")).unwrap();
        fs::create_dir_all(workspace_dir.join("reply_email_attachments")).unwrap();
        fs::write(workspace_dir.join(".task_stdout.log"), "streamed output").unwrap();
        fs::write(
            workspace_dir.join("reply_email_draft.html"),
            "<p>hello frontend</p>",
        )
        .unwrap();
        fs::write(
            workspace_dir.join("reply_email_attachments").join("note.txt"),
            "attachment",
        )
        .unwrap();

        let envelope = TaskEnvelope {
            task: QueuedTask {
                id: "demo-task".to_string(),
                created_at: Utc::now(),
                status: TaskStatus::Completed,
                request: InboundTaskRequest {
                    customer_email: "dylan@example.com".to_string(),
                    subject: "Frontend test".to_string(),
                    prompt: "Run".to_string(),
                    channel: "email".to_string(),
                    reply_to: "reply@example.com".to_string(),
                    tenant_id: String::new(),
                    account_id: String::new(),
                    memory_uri: String::new(),
                    identity_uri: String::new(),
                    credential_refs: Vec::new(),
                },
                workspace_key: "default-tenant/dylan_example_com/demo-task".to_string(),
                workspace_dir: workspace_dir.display().to_string(),
            },
            worker_id: Some("worker-1".to_string()),
            error: None,
        };

        fs::write(
            queue_root.join("completed").join("demo-task.json"),
            serde_json::to_string_pretty(&envelope).unwrap(),
        )
        .unwrap();

        let snapshot = TaskInspector::new(&queue_root)
            .get("demo-task")
            .unwrap()
            .unwrap();

        assert_eq!(snapshot.status, TaskStatus::Completed);
        assert_eq!(snapshot.stdout, "streamed output");
        assert_eq!(snapshot.reply_html.as_deref(), Some("<p>hello frontend</p>"));
        assert_eq!(snapshot.attachment_names, vec!["note.txt"]);
    }

    fn temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{}-{}", prefix, unique));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
