use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::models::{QueuedTask, TaskEnvelope, TaskStatus};

#[derive(Debug, Clone)]
pub struct FileQueue {
    root: PathBuf,
}

impl FileQueue {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let queue = Self { root: root.into() };
        queue.ensure_layout()?;
        Ok(queue)
    }

    pub fn enqueue(&self, task: &QueuedTask) -> Result<()> {
        let envelope = TaskEnvelope {
            task: task.clone(),
            worker_id: None,
            error: None,
        };
        let queue_path = self.pending_path(&task.id);
        self.write_envelope(&queue_path, &envelope)?;
        tracing::info!(
            task_id = %task.id,
            workspace_key = %task.workspace_key,
            queue_path = %queue_path.display(),
            "enqueued task"
        );
        Ok(())
    }

    pub fn claim_next(&self, worker_id: &str) -> Result<Option<TaskEnvelope>> {
        let mut entries = self.read_dir_sorted(&self.pending_dir())?;
        while let Some(path) = entries.pop() {
            let file_name = match path.file_name().and_then(|value| value.to_str()) {
                Some(value) => value.to_string(),
                None => continue,
            };
            let claimed_path = self.claimed_dir().join(file_name);
            match fs::rename(&path, &claimed_path) {
                Ok(()) => {
                    let mut envelope = self.read_envelope(&claimed_path)?;
                    envelope.task.status = TaskStatus::Claimed;
                    envelope.worker_id = Some(worker_id.to_string());
                    self.write_envelope(&claimed_path, &envelope)?;
                    tracing::info!(
                        task_id = %envelope.task.id,
                        worker_id = worker_id,
                        claimed_path = %claimed_path.display(),
                        "claimed task for worker"
                    );
                    return Ok(Some(envelope));
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => return Err(err).context("failed to claim pending task"),
            }
        }
        Ok(None)
    }

    pub fn complete(&self, mut envelope: TaskEnvelope) -> Result<()> {
        envelope.task.status = TaskStatus::Completed;
        self.write_terminal(&self.completed_dir(), envelope.clone())?;
        tracing::info!(
            task_id = %envelope.task.id,
            worker_id = envelope.worker_id.as_deref().unwrap_or("unknown"),
            "marked task completed"
        );
        Ok(())
    }

    pub fn fail(&self, mut envelope: TaskEnvelope, error: String) -> Result<()> {
        envelope.task.status = TaskStatus::Failed;
        envelope.error = Some(error);
        self.write_terminal(&self.failed_dir(), envelope.clone())?;
        tracing::warn!(
            task_id = %envelope.task.id,
            worker_id = envelope.worker_id.as_deref().unwrap_or("unknown"),
            error = %envelope.error.as_deref().unwrap_or("unknown"),
            "marked task failed"
        );
        Ok(())
    }

    fn write_terminal(&self, dir: &Path, envelope: TaskEnvelope) -> Result<()> {
        let claimed_path = self.claimed_path(&envelope.task.id);
        let target_path = dir.join(format!("{}.json", envelope.task.id));
        self.write_envelope(&target_path, &envelope)?;
        if claimed_path.exists() {
            fs::remove_file(claimed_path)?;
        }
        Ok(())
    }

    fn read_dir_sorted(&self, dir: &Path) -> Result<Vec<PathBuf>> {
        let mut entries = fs::read_dir(dir)?
            .filter_map(|entry| entry.ok().map(|value| value.path()))
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        entries.sort();
        entries.reverse();
        Ok(entries)
    }

    fn read_envelope(&self, path: &Path) -> Result<TaskEnvelope> {
        let contents = fs::read_to_string(path)?;
        serde_json::from_str(&contents).context("failed to parse queue envelope")
    }

    fn write_envelope(&self, path: impl AsRef<Path>, envelope: &TaskEnvelope) -> Result<()> {
        let contents = serde_json::to_string_pretty(envelope)?;
        fs::write(path, contents)?;
        Ok(())
    }

    fn pending_path(&self, task_id: &str) -> PathBuf {
        self.pending_dir().join(format!("{}.json", task_id))
    }

    fn claimed_path(&self, task_id: &str) -> PathBuf {
        self.claimed_dir().join(format!("{}.json", task_id))
    }

    fn ensure_layout(&self) -> Result<()> {
        fs::create_dir_all(self.pending_dir())?;
        fs::create_dir_all(self.claimed_dir())?;
        fs::create_dir_all(self.completed_dir())?;
        fs::create_dir_all(self.failed_dir())?;
        Ok(())
    }

    fn pending_dir(&self) -> PathBuf {
        self.root.join("pending")
    }

    fn claimed_dir(&self) -> PathBuf {
        self.root.join("claimed")
    }

    fn completed_dir(&self) -> PathBuf {
        self.root.join("completed")
    }

    fn failed_dir(&self) -> PathBuf {
        self.root.join("failed")
    }
}
