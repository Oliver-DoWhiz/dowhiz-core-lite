use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::InboundTaskRequest;

#[derive(Debug, Clone)]
pub struct WorkspaceLayout {
    pub workspace_key: String,
    pub workspace_dir: PathBuf,
    tenant_key: String,
    account_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceManifest {
    pub task_id: String,
    pub created_at: DateTime<Utc>,
    pub workspace_key: String,
    pub workspace_dir: String,
    pub tenant_id: String,
    pub account_id: String,
    pub customer_email: String,
    pub subject: String,
    pub memory_uri: Option<String>,
    pub identity_uri: Option<String>,
    pub credential_refs: Vec<String>,
}

pub fn plan_workspace(
    tasks_root: &Path,
    task_id: &str,
    request: &InboundTaskRequest,
) -> WorkspaceLayout {
    let tenant_key = sanitize_segment(&request.tenant_id, "default-tenant");
    let account_source = if request.account_id.trim().is_empty() {
        &request.customer_email
    } else {
        &request.account_id
    };
    let account_key = sanitize_segment(account_source, "anonymous");
    let workspace_key = format!("{}/{}/{}", tenant_key, account_key, task_id);
    let workspace_dir = tasks_root.join(&tenant_key).join(&account_key).join(task_id);

    WorkspaceLayout {
        workspace_key,
        workspace_dir,
        tenant_key,
        account_key,
    }
}

pub fn initialize_workspace(
    layout: &WorkspaceLayout,
    task_id: &str,
    created_at: DateTime<Utc>,
    request: &InboundTaskRequest,
) -> Result<WorkspaceManifest> {
    fs::create_dir_all(layout.workspace_dir.join("incoming_email"))?;
    fs::create_dir_all(layout.workspace_dir.join("incoming_attachments"))?;
    fs::create_dir_all(layout.workspace_dir.join("reply_email_attachments"))?;

    fs::write(
        layout.workspace_dir.join("incoming_email/thread_request.md"),
        render_thread_request(request),
    )?;
    fs::write(
        layout.workspace_dir.join("task_request.json"),
        serde_json::to_string_pretty(request)?,
    )?;

    let manifest = WorkspaceManifest {
        task_id: task_id.to_string(),
        created_at,
        workspace_key: layout.workspace_key.clone(),
        workspace_dir: layout.workspace_dir.display().to_string(),
        tenant_id: layout.tenant_key.clone(),
        account_id: layout.account_key.clone(),
        customer_email: request.customer_email.clone(),
        subject: request.subject.clone(),
        memory_uri: optional_string(&request.memory_uri),
        identity_uri: optional_string(&request.identity_uri),
        credential_refs: request.credential_refs.clone(),
    };

    fs::write(
        layout.workspace_dir.join("workspace_manifest.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;

    Ok(manifest)
}

fn render_thread_request(request: &InboundTaskRequest) -> String {
    format!(
        concat!(
            "# Incoming request\n\n",
            "From: {}\n",
            "Subject: {}\n",
            "Channel: {}\n",
            "Reply-To: {}\n",
            "Tenant-ID: {}\n",
            "Account-ID: {}\n",
            "Memory-URI: {}\n",
            "Identity-URI: {}\n",
            "Credential-Refs: {}\n\n",
            "## Prompt\n{}\n"
        ),
        request.customer_email,
        request.subject,
        request.channel,
        request.reply_to,
        if request.tenant_id.trim().is_empty() {
            "default-tenant"
        } else {
            request.tenant_id.trim()
        },
        if request.account_id.trim().is_empty() {
            request.customer_email.trim()
        } else {
            request.account_id.trim()
        },
        empty_placeholder(&request.memory_uri),
        empty_placeholder(&request.identity_uri),
        if request.credential_refs.is_empty() {
            "(none)".to_string()
        } else {
            request.credential_refs.join(", ")
        },
        request.prompt
    )
}

fn sanitize_segment(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }

    let sanitized = trimmed
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            '@' | '.' | '/' | ':' => '_',
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    if sanitized.is_empty() {
        fallback.to_string()
    } else {
        sanitized
    }
}

fn empty_placeholder(value: &str) -> &str {
    if value.trim().is_empty() {
        "(none)"
    } else {
        value.trim()
    }
}

fn optional_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::InboundTaskRequest;

    #[test]
    fn partitions_workspace_by_tenant_and_account() {
        let request = InboundTaskRequest {
            customer_email: "dylan@example.com".to_string(),
            subject: "Test".to_string(),
            prompt: "Hello".to_string(),
            channel: "email".to_string(),
            reply_to: "reply@example.com".to_string(),
            tenant_id: "prod/us".to_string(),
            account_id: "user:42".to_string(),
            memory_uri: String::new(),
            identity_uri: String::new(),
            credential_refs: Vec::new(),
        };

        let layout = plan_workspace(Path::new(".workspace/tasks"), "task-123", &request);

        assert_eq!(layout.workspace_key, "prod_us/user_42/task-123");
        assert_eq!(
            layout.workspace_dir,
            PathBuf::from(".workspace/tasks/prod_us/user_42/task-123")
        );
    }
}
