use std::env;
use std::fs;
use std::process::Command;

use anyhow::{anyhow, Result};

use crate::types::PreparedWorkspace;

pub fn run_locally(prepared: &PreparedWorkspace) -> Result<String> {
    if let Ok(command) = env::var("LOCAL_AGENT_COMMAND") {
        let output = Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(&prepared.workspace_dir)
            .output()?;

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
