use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::types::PreparedWorkspace;

pub fn prepare_workspace(workspace_dir: &Path, prompt: &str) -> Result<PreparedWorkspace> {
    fs::create_dir_all(workspace_dir)?;

    let prompt_path = workspace_dir.join("task_prompt.txt");
    let stdout_path = workspace_dir.join(".task_stdout.log");
    let reply_html_path = workspace_dir.join("reply_email_draft.html");
    let reply_attachments_dir = workspace_dir.join("reply_email_attachments");

    fs::create_dir_all(&reply_attachments_dir)?;
    fs::write(&prompt_path, prompt)?;

    Ok(PreparedWorkspace {
        workspace_dir: workspace_dir.to_path_buf(),
        prompt_path,
        stdout_path,
        reply_html_path,
        reply_attachments_dir,
        prompt: prompt.to_string(),
    })
}

pub fn write_reply_html(path: &Path, stdout: &str) -> Result<()> {
    let html = format!(
        "<html><body><h1>Task Result</h1><pre>{}</pre></body></html>",
        escape_html(stdout)
    );
    fs::write(path, html)?;
    Ok(())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
