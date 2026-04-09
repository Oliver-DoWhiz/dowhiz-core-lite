use std::env;
use std::fs;
use std::process::Command;

use anyhow::{anyhow, Result};

use crate::types::PreparedWorkspace;

pub fn run_locally(prepared: &PreparedWorkspace) -> Result<String> {
    if let Ok(command) = env::var("LOCAL_AGENT_COMMAND") {
        let mut shell = Command::new("sh");
        shell
            .arg("-lc")
            .arg(command)
            .current_dir(&prepared.workspace_dir)
            .env("TASK_WORKSPACE_DIR", &prepared.workspace_dir)
            .env("TASK_PROMPT_FILE", &prepared.prompt_path)
            .env("TASK_OUTPUT_FILE", &prepared.stdout_path)
            .env("TASK_REPLY_HTML_FILE", &prepared.reply_html_path)
            .env("TASK_REPLY_ATTACHMENTS_DIR", &prepared.reply_attachments_dir)
            .env("TASK_METADATA_FILE", &prepared.manifest_path);

        if prepared.secrets_env_path.exists() {
            for (key, value) in read_env_pairs(&prepared.secrets_env_path)? {
                shell.env(key, value);
            }
        }

        let output = shell.output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            return Err(anyhow!(
                "local agent command failed: {}",
                if stderr.is_empty() { "unknown error" } else { &stderr }
            ));
        }

        fs::write(&prepared.stdout_path, &stdout)?;
        return Ok(stdout);
    }

    let synthesized = format!(
        "Lightweight DoWhiz worker processed the request.\n\nPrompt:\n{}\n",
        prepared.prompt
    );
    fs::write(&prepared.stdout_path, &synthesized)?;
    Ok(synthesized)
}

fn read_env_pairs(path: &std::path::Path) -> Result<Vec<(String, String)>> {
    let contents = fs::read_to_string(path)?;
    let mut vars = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        vars.push((key.trim().to_string(), value.trim().to_string()));
    }

    Ok(vars)
}
