use std::fs;
use std::path::Path;

use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::models::InboundTaskRequest;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PostmarkInboundPayload {
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub from_name: String,
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text_body: String,
    #[serde(default)]
    pub html_body: String,
    #[serde(default)]
    pub stripped_text_reply: String,
    #[serde(default)]
    pub reply_to: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub attachments: Vec<PostmarkInboundAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PostmarkInboundAttachment {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub content_length: usize,
    #[serde(default)]
    pub content: String,
}

pub fn task_request_from_postmark(payload: &PostmarkInboundPayload) -> InboundTaskRequest {
    let customer_email = extract_sender_email(&payload.from);
    let reply_to = if payload.reply_to.trim().is_empty() {
        customer_email.clone()
    } else {
        extract_sender_email(&payload.reply_to)
    };

    InboundTaskRequest {
        customer_email: customer_email.clone(),
        subject: payload.subject.clone(),
        prompt: render_prompt(payload, &customer_email),
        channel: "email".to_string(),
        reply_to,
        tenant_id: String::new(),
        account_id: String::new(),
        memory_uri: String::new(),
        identity_uri: String::new(),
        credential_refs: Vec::new(),
    }
}

pub fn persist_postmark_inbound_artifacts(
    workspace_dir: &Path,
    payload: &PostmarkInboundPayload,
    request: &InboundTaskRequest,
) -> Result<()> {
    let email_dir = workspace_dir.join("incoming_email");
    let attachments_dir = workspace_dir.join("incoming_attachments");

    fs::create_dir_all(&email_dir)?;
    fs::create_dir_all(&attachments_dir)?;

    fs::write(
        email_dir.join("postmark_payload.json"),
        serde_json::to_string_pretty(payload)?,
    )?;
    fs::write(
        email_dir.join("thread_request.md"),
        render_thread_request(payload, request),
    )?;
    fs::write(email_dir.join("email.html"), render_email_html(payload))?;

    let mut attachment_names = Vec::new();
    for attachment in &payload.attachments {
        let file_name = sanitize_file_name(&attachment.name);
        if file_name.is_empty() {
            continue;
        }

        attachment_names.push(file_name.clone());
        if attachment.content.trim().is_empty() {
            continue;
        }

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(attachment.content.as_bytes())
            .map_err(|err| anyhow!("failed to decode inbound attachment {}: {}", file_name, err))?;
        fs::write(attachments_dir.join(&file_name), bytes)?;
    }

    fs::write(
        attachments_dir.join("thread_manifest.json"),
        serde_json::to_string_pretty(&AttachmentManifest {
            attachment_names,
        })?,
    )?;

    Ok(())
}

fn render_prompt(payload: &PostmarkInboundPayload, customer_email: &str) -> String {
    let message_body = if payload.stripped_text_reply.trim().is_empty() {
        payload.text_body.trim()
    } else {
        payload.stripped_text_reply.trim()
    };

    let attachment_summary = if payload.attachments.is_empty() {
        "(none)".to_string()
    } else {
        payload
            .attachments
            .iter()
            .map(|attachment| sanitize_file_name(&attachment.name))
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        concat!(
            "You received a new inbound email task.\n\n",
            "Sender: {}\n",
            "From header: {}\n",
            "Subject: {}\n",
            "Reply-To: {}\n",
            "Date: {}\n",
            "Message-ID: {}\n",
            "Attachments: {}\n\n",
            "Reply helpfully to the sender after completing their request.\n\n",
            "Email body:\n{}\n"
        ),
        customer_email,
        payload.from.trim(),
        payload.subject.trim(),
        empty_placeholder(&payload.reply_to),
        empty_placeholder(&payload.date),
        empty_placeholder(&payload.message_id),
        attachment_summary,
        message_body
    )
}

fn render_thread_request(payload: &PostmarkInboundPayload, request: &InboundTaskRequest) -> String {
    let attachment_summary = if payload.attachments.is_empty() {
        "(none)".to_string()
    } else {
        payload
            .attachments
            .iter()
            .map(|attachment| sanitize_file_name(&attachment.name))
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    };

    format!(
        concat!(
            "# Canonical thread request\n\n",
            "Channel: email\n",
            "Provider: postmark_inbound\n",
            "From: {}\n",
            "Reply-To: {}\n",
            "To: {}\n",
            "Cc: {}\n",
            "Subject: {}\n",
            "Date: {}\n",
            "Message-ID: {}\n",
            "Customer-Email: {}\n",
            "Attachments: {}\n\n",
            "## Latest inbound message\n{}\n"
        ),
        payload.from.trim(),
        empty_placeholder(&payload.reply_to),
        empty_placeholder(&payload.to),
        empty_placeholder(&payload.cc),
        payload.subject.trim(),
        empty_placeholder(&payload.date),
        empty_placeholder(&payload.message_id),
        request.customer_email.trim(),
        attachment_summary,
        if payload.text_body.trim().is_empty() {
            "(empty)"
        } else {
            payload.text_body.trim()
        }
    )
}

fn render_email_html(payload: &PostmarkInboundPayload) -> String {
    if !payload.html_body.trim().is_empty() {
        payload.html_body.clone()
    } else {
        format!(
            "<html><body><pre>{}</pre></body></html>",
            escape_html(&payload.text_body)
        )
    }
}

fn extract_sender_email(value: &str) -> String {
    let trimmed = value.trim();
    if let Some((_, rest)) = trimmed.split_once('<') {
        if let Some((email, _)) = rest.split_once('>') {
            return email.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn sanitize_file_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn empty_placeholder(value: &str) -> &str {
    if value.trim().is_empty() {
        "(none)"
    } else {
        value.trim()
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[derive(Debug, Serialize)]
struct AttachmentManifest {
    attachment_names: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn builds_task_request_from_postmark_payload() {
        let payload = PostmarkInboundPayload {
            from: "Dylan Tang <dtang04@uchicago.edu>".to_string(),
            from_name: "Dylan Tang".to_string(),
            to: "oliver@dowhiz.com".to_string(),
            cc: String::new(),
            subject: "Need help".to_string(),
            text_body: "Please review the repo.".to_string(),
            html_body: String::new(),
            stripped_text_reply: String::new(),
            reply_to: String::new(),
            message_id: "mid-1".to_string(),
            date: "2026-04-09".to_string(),
            attachments: Vec::new(),
        };

        let request = task_request_from_postmark(&payload);

        assert_eq!(request.customer_email, "dtang04@uchicago.edu");
        assert_eq!(request.reply_to, "dtang04@uchicago.edu");
        assert!(request.prompt.contains("Please review the repo."));
    }

    #[test]
    fn persists_inbound_artifacts_and_decodes_attachments() {
        let root = temp_dir("postmark-inbound");
        let payload = PostmarkInboundPayload {
            from: "dtang04@uchicago.edu".to_string(),
            from_name: String::new(),
            to: "oliver@dowhiz.com".to_string(),
            cc: String::new(),
            subject: "Need help".to_string(),
            text_body: "Body".to_string(),
            html_body: String::new(),
            stripped_text_reply: String::new(),
            reply_to: String::new(),
            message_id: "mid-2".to_string(),
            date: "2026-04-09".to_string(),
            attachments: vec![PostmarkInboundAttachment {
                name: "notes.txt".to_string(),
                content_type: "text/plain".to_string(),
                content_length: 5,
                content: base64::engine::general_purpose::STANDARD.encode("hello"),
            }],
        };
        let request = task_request_from_postmark(&payload);

        persist_postmark_inbound_artifacts(&root, &payload, &request).unwrap();

        assert!(root.join("incoming_email/postmark_payload.json").exists());
        assert_eq!(
            fs::read_to_string(root.join("incoming_attachments/notes.txt")).unwrap(),
            "hello"
        );
    }

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{}-{}", prefix, unique));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
