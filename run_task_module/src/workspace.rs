use std::fs;
use std::path::Path;

use anyhow::Result;

use crate::prompt::{render_task_prompt, system_prompt, SYSTEM_PROMPT_FILE_NAME};
use crate::types::PreparedWorkspace;

pub fn prepare_workspace(workspace_dir: &Path, prompt: &str) -> Result<PreparedWorkspace> {
    fs::create_dir_all(workspace_dir)?;
    let workspace_dir = workspace_dir.canonicalize()?;

    let prompt_path = workspace_dir.join("task_prompt.txt");
    let system_prompt_path = workspace_dir.join(SYSTEM_PROMPT_FILE_NAME);
    let stdout_path = workspace_dir.join(".task_stdout.log");
    let reply_html_path = workspace_dir.join("reply_email_draft.html");
    let reply_attachments_dir = workspace_dir.join("reply_email_attachments");
    let manifest_path = workspace_dir.join("workspace_manifest.json");
    let secrets_env_path = workspace_dir.join(".task_secrets.env");
    let rendered_prompt = render_task_prompt(prompt);

    fs::create_dir_all(&reply_attachments_dir)?;
    fs::write(&system_prompt_path, system_prompt())?;
    fs::write(&prompt_path, &rendered_prompt)?;

    Ok(PreparedWorkspace {
        workspace_dir: workspace_dir.to_path_buf(),
        prompt_path,
        stdout_path,
        reply_html_path,
        reply_attachments_dir,
        manifest_path,
        secrets_env_path,
        prompt: rendered_prompt,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::prepare_workspace;

    #[test]
    fn writes_system_prompt_and_combined_task_prompt() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let workspace_dir = std::env::temp_dir().join(format!("dowhiz-core-lite-workspace-{unique}"));

        let prepared = prepare_workspace(&workspace_dir, "Handle the inbound request.").unwrap();
        let system_prompt = fs::read_to_string(workspace_dir.join("codex_system_prompt.md")).unwrap();
        let task_prompt = fs::read_to_string(&prepared.prompt_path).unwrap();

        assert!(system_prompt.contains("## Workspace contract"));
        assert!(task_prompt.contains("## User Request"));
        assert!(task_prompt.contains("Handle the inbound request."));

        fs::remove_dir_all(&workspace_dir).unwrap();
    }
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
