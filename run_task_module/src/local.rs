use std::env;
use std::fs;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::process::Command;
use std::process::Stdio;
use std::thread;

use anyhow::{anyhow, Context, Result};

use crate::types::PreparedWorkspace;

pub fn run_locally(prepared: &PreparedWorkspace) -> Result<String> {
    if let Some(command) = resolve_agent_command() {
        tracing::info!(
            workspace_dir = %prepared.workspace_dir.display(),
            command = %command,
            "resolved local agent command"
        );
        return run_command(prepared, &command);
    }

    let synthesized = format!(
        "Lightweight DoWhiz worker processed the request.\n\nPrompt:\n{}\n",
        prepared.prompt
    );
    fs::write(&prepared.stdout_path, &synthesized)?;
    tracing::info!(
        workspace_dir = %prepared.workspace_dir.display(),
        stdout_path = %prepared.stdout_path.display(),
        "no local agent command available; wrote synthesized task output"
    );
    Ok(synthesized)
}

fn run_command(prepared: &PreparedWorkspace, command: &str) -> Result<String> {
    let secret_env = if prepared.secrets_env_path.exists() {
        read_env_pairs(&prepared.secrets_env_path)?
    } else {
        Vec::new()
    };

    tracing::info!(
        workspace_dir = %prepared.workspace_dir.display(),
        command = %command,
        prompt_path = %prepared.prompt_path.display(),
        output_path = %prepared.stdout_path.display(),
        reply_html_path = %prepared.reply_html_path.display(),
        secret_env_vars = secret_env.len(),
        "starting local agent process"
    );
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
        .env("TASK_METADATA_FILE", &prepared.manifest_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in secret_env {
        shell.env(key, value);
    }

    let mut child = shell.spawn()?;
    let stdout = child.stdout.take().context("missing stdout pipe for local agent command")?;
    let stderr = child.stderr.take().context("missing stderr pipe for local agent command")?;

    let stdout_path = prepared.stdout_path.clone();
    let stdout_handle = thread::spawn(move || -> Result<Vec<u8>> {
        let mut reader = BufReader::new(stdout);
        let mut file = File::create(stdout_path)?;
        let mut collected = Vec::new();
        let mut chunk = [0_u8; 4096];

        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            file.write_all(&chunk[..read])?;
            file.flush()?;
            collected.extend_from_slice(&chunk[..read]);
        }

        Ok(collected)
    });

    let stderr_handle = thread::spawn(move || -> Result<Vec<u8>> {
        let mut reader = BufReader::new(stderr);
        let mut collected = Vec::new();
        reader.read_to_end(&mut collected)?;
        Ok(collected)
    });

    let status = child.wait()?;
    let stdout = stdout_handle
        .join()
        .map_err(|_| anyhow!("failed to join stdout reader thread"))??;
    let stderr = stderr_handle
        .join()
        .map_err(|_| anyhow!("failed to join stderr reader thread"))??;

    let stdout = String::from_utf8_lossy(&stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&stderr).trim().to_string();

    if !status.success() {
        return Err(anyhow!(
            "local agent command failed: {}",
            if stderr.is_empty() { "unknown error" } else { &stderr }
        ));
    }

    tracing::info!(
        workspace_dir = %prepared.workspace_dir.display(),
        exit_code = status.code().unwrap_or_default(),
        stdout_bytes = stdout.len(),
        stderr_bytes = stderr.len(),
        "local agent process completed"
    );
    Ok(stdout)
}

fn resolve_agent_command() -> Option<String> {
    if let Ok(command) = env::var("LOCAL_AGENT_COMMAND") {
        let trimmed = command.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if command_exists("codex") {
        return Some("cat \"$TASK_PROMPT_FILE\" | codex exec -".to_string());
    }

    None
}

fn command_exists(binary: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&path).any(|dir| dir.join(binary).is_file())
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
