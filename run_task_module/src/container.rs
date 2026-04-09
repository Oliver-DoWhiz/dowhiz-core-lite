use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Result};

use crate::types::PreparedWorkspace;

pub fn run_in_container(prepared: &PreparedWorkspace, image: &str) -> Result<String> {
    let workspace = canonicalize(&prepared.workspace_dir)?;
    let prompt_file = "/workspace/task_prompt.txt";
    let stdout_file = "/workspace/.task_stdout.log";

    let output = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("-v")
        .arg(format!("{}:/workspace", workspace.display()))
        .arg("-e")
        .arg(format!("TASK_PROMPT_FILE={}", prompt_file))
        .arg("-e")
        .arg(format!("TASK_OUTPUT_FILE={}", stdout_file))
        .arg(image)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("container runner failed: {}", stderr.trim()));
    }

    let stdout = fs::read_to_string(&prepared.stdout_path)?;
    Ok(stdout)
}

fn canonicalize(path: &PathBuf) -> Result<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Err(anyhow!("workspace does not exist: {}", path.display()))
    }
}
