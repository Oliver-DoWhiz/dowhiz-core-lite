use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundPreview {
    pub subject: String,
    pub html_body: String,
    pub attachment_names: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct OutboundMessage {
    pub from: String,
    pub to: String,
    pub subject: String,
    pub html_body: String,
    pub reply_to: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PostmarkConfig {
    pub api_base_url: String,
    pub server_token: String,
    pub message_stream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryReport {
    pub provider: String,
    pub subject: String,
    pub to: String,
    pub attachment_names: Vec<String>,
    pub message_id: Option<String>,
    pub submitted_at: Option<String>,
    pub raw_response: serde_json::Value,
}

pub fn build_outbound_preview(
    reply_html_path: &Path,
    attachments_dir: &Path,
    subject: String,
) -> Result<OutboundPreview> {
    let html_body = fs::read_to_string(reply_html_path)?;
    let attachment_names = list_attachment_names(attachments_dir)?;

    Ok(OutboundPreview {
        subject,
        html_body,
        attachment_names,
    })
}

pub fn write_preview_json(path: PathBuf, preview: &OutboundPreview) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(preview)?)?;
    Ok(())
}

pub fn write_delivery_report(path: PathBuf, report: &DeliveryReport) -> Result<()> {
    fs::write(path, serde_json::to_string_pretty(report)?)?;
    Ok(())
}

pub fn send_via_postmark(
    config: &PostmarkConfig,
    message: &OutboundMessage,
    attachments_dir: &Path,
) -> Result<DeliveryReport> {
    let payload = build_postmark_request(config, message, attachments_dir)?;
    let response = reqwest::blocking::Client::new()
        .post(format!("{}/email", config.api_base_url.trim_end_matches('/')))
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("X-Postmark-Server-Token", &config.server_token)
        .json(&payload)
        .send()
        .context("failed to call Postmark")?;

    let status = response.status();
    let raw_response: serde_json::Value = response
        .json()
        .context("failed to parse Postmark response body")?;

    if !status.is_success() {
        return Err(anyhow!(
            "Postmark send failed with status {}: {}",
            status,
            raw_response
        ));
    }

    let provider_response: PostmarkSendResponse = serde_json::from_value(raw_response.clone())
        .context("failed to parse Postmark success payload")?;

    Ok(DeliveryReport {
        provider: "postmark".to_string(),
        subject: message.subject.clone(),
        to: message.to.clone(),
        attachment_names: list_attachment_names(attachments_dir)?,
        message_id: provider_response.message_id,
        submitted_at: provider_response.submitted_at,
        raw_response,
    })
}

fn build_postmark_request(
    config: &PostmarkConfig,
    message: &OutboundMessage,
    attachments_dir: &Path,
) -> Result<PostmarkSendRequest> {
    Ok(PostmarkSendRequest {
        from: message.from.clone(),
        to: message.to.clone(),
        subject: message.subject.clone(),
        html_body: message.html_body.clone(),
        reply_to: message.reply_to.clone(),
        tag: message.tag.clone(),
        message_stream: config.message_stream.clone(),
        attachments: load_attachments(attachments_dir)?,
    })
}

fn load_attachments(dir: &Path) -> Result<Vec<PostmarkAttachment>> {
    let mut attachments = Vec::new();
    if !dir.exists() {
        return Ok(attachments);
    }

    let mut entries = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("attachment file name is not valid UTF-8"))?
            .to_string();
        let content_type = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .essence_str()
            .to_string();
        let bytes = fs::read(&path)?;
        let content = base64::engine::general_purpose::STANDARD.encode(bytes);

        attachments.push(PostmarkAttachment {
            name,
            content,
            content_type,
            content_id: None,
        });
    }

    Ok(attachments)
}

fn list_attachment_names(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut names = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
struct PostmarkSendRequest {
    from: String,
    to: String,
    subject: String,
    html_body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_stream: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    attachments: Vec<PostmarkAttachment>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "PascalCase")]
struct PostmarkAttachment {
    name: String,
    content: String,
    content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct PostmarkSendResponse {
    #[serde(default)]
    message_id: Option<String>,
    #[serde(default)]
    submitted_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn builds_preview_and_attachment_listing() {
        let root = temp_dir("preview");
        let attachments_dir = root.join("attachments");
        fs::create_dir_all(&attachments_dir).unwrap();
        fs::write(root.join("reply.html"), "<p>Hello</p>").unwrap();
        fs::write(attachments_dir.join("b.txt"), "b").unwrap();
        fs::write(attachments_dir.join("a.txt"), "a").unwrap();

        let preview =
            build_outbound_preview(&root.join("reply.html"), &attachments_dir, "Hello".into())
                .unwrap();

        assert_eq!(preview.subject, "Hello");
        assert_eq!(preview.attachment_names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn encodes_postmark_attachments() {
        let root = temp_dir("postmark");
        let attachments_dir = root.join("attachments");
        fs::create_dir_all(&attachments_dir).unwrap();
        fs::write(attachments_dir.join("note.txt"), "hello").unwrap();

        let payload = build_postmark_request(
            &PostmarkConfig {
                api_base_url: "https://api.postmarkapp.com".to_string(),
                server_token: "token".to_string(),
                message_stream: Some("outbound".to_string()),
            },
            &OutboundMessage {
                from: "from@example.com".to_string(),
                to: "to@example.com".to_string(),
                subject: "Subject".to_string(),
                html_body: "<p>Hello</p>".to_string(),
                reply_to: Some("reply@example.com".to_string()),
                tag: Some("task-result".to_string()),
            },
            &attachments_dir,
        )
        .unwrap();

        assert_eq!(payload.attachments.len(), 1);
        assert_eq!(payload.attachments[0].name, "note.txt");
        assert_eq!(payload.message_stream.as_deref(), Some("outbound"));
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
