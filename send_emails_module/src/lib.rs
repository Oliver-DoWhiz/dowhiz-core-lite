use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundPreview {
    pub subject: String,
    pub html_body: String,
    pub attachment_names: Vec<String>,
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
