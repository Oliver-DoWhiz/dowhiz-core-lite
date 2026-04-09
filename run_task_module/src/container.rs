use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};

use crate::types::{ContainerMode, PreparedWorkspace, RunTaskParams};

const DEFAULT_WORKSPACE_MOUNT: &str = "/workspace";

pub fn run_in_container(prepared: &PreparedWorkspace, params: &RunTaskParams) -> Result<String> {
    match params.container_mode {
        ContainerMode::OneShot => run_one_shot(prepared, params)?,
        ContainerMode::WarmPool => run_warm_pool(prepared, params)?,
    }

    let stdout = fs::read_to_string(&prepared.stdout_path)?;
    Ok(stdout)
}

fn run_one_shot(prepared: &PreparedWorkspace, params: &RunTaskParams) -> Result<()> {
    let workspace = canonicalize(&prepared.workspace_dir)?;
    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("--rm")
        .arg("-v")
        .arg(format!("{}:{}", workspace.display(), DEFAULT_WORKSPACE_MOUNT));

    append_passthrough_env(&mut command, &params.env_passthrough);
    append_secret_env(&mut command, &prepared.secrets_env_path)?;
    append_task_env(
        &mut command,
        DEFAULT_WORKSPACE_MOUNT,
        DEFAULT_WORKSPACE_MOUNT,
    );

    let output = command.arg(&params.container_image).output()?;
    ensure_success(output.status.success(), &output.stderr, "container runner")?;
    Ok(())
}

fn run_warm_pool(prepared: &PreparedWorkspace, params: &RunTaskParams) -> Result<()> {
    let workspace = canonicalize(&prepared.workspace_dir)?;
    let host_root = params
        .container_host_root
        .as_ref()
        .context("warm container mode requires RUN_TASK_CONTAINER_HOST_ROOT")?;
    let host_root = canonicalize(host_root)?;
    let relative_workspace = workspace
        .strip_prefix(&host_root)
        .with_context(|| {
            format!(
                "workspace {} is not inside warm pool root {}",
                workspace.display(),
                host_root.display()
            )
        })?;

    let pool_name = pool_name(&params.container_pool_name, &host_root, &params.container_image);
    ensure_pool_container(&pool_name, &host_root, params)?;

    let container_workspace_root = normalize_container_root(&params.container_workspace_root);
    let container_workspace = join_container_path(&container_workspace_root, relative_workspace);

    let mut command = Command::new("docker");
    command.arg("exec");
    append_passthrough_env(&mut command, &params.env_passthrough);
    append_secret_env(&mut command, &prepared.secrets_env_path)?;
    append_task_env(&mut command, &container_workspace, &container_workspace_root);
    let output = command
        .arg(pool_name)
        .arg("/app/exec_codex.sh")
        .output()?;

    ensure_success(output.status.success(), &output.stderr, "warm container exec")?;
    Ok(())
}

fn ensure_pool_container(pool_name: &str, host_root: &Path, params: &RunTaskParams) -> Result<()> {
    let inspect = Command::new("docker")
        .arg("inspect")
        .arg("-f")
        .arg("{{.State.Running}}")
        .arg(pool_name)
        .output()?;

    if inspect.status.success() && String::from_utf8_lossy(&inspect.stdout).trim() == "true" {
        return Ok(());
    }

    if inspect.status.success() {
        let rm_output = Command::new("docker").arg("rm").arg("-f").arg(pool_name).output()?;
        ensure_success(
            rm_output.status.success(),
            &rm_output.stderr,
            "warm pool cleanup",
        )?;
    }

    let container_root = normalize_container_root(&params.container_workspace_root);
    let output = Command::new("docker")
        .arg("run")
        .arg("-d")
        .arg("--rm")
        .arg("--name")
        .arg(pool_name)
        .arg("-e")
        .arg("CODEX_RUNNER_MODE=pool")
        .arg("-v")
        .arg(format!("{}:{}", host_root.display(), container_root))
        .arg(&params.container_image)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool bootstrap",
    )?;

    Ok(())
}

fn append_task_env(command: &mut Command, workspace_dir: &str, container_root: &str) {
    let output_file = format!("{}/.task_stdout.log", workspace_dir);
    let prompt_file = format!("{}/task_prompt.txt", workspace_dir);
    let reply_html_file = format!("{}/reply_email_draft.html", workspace_dir);
    let attachments_dir = format!("{}/reply_email_attachments", workspace_dir);
    let metadata_file = format!("{}/workspace_manifest.json", workspace_dir);

    command
        .arg("-e")
        .arg(format!("TASK_WORKSPACE_DIR={}", workspace_dir))
        .arg("-e")
        .arg(format!("TASK_PROMPT_FILE={}", prompt_file))
        .arg("-e")
        .arg(format!("TASK_OUTPUT_FILE={}", output_file))
        .arg("-e")
        .arg(format!("TASK_REPLY_HTML_FILE={}", reply_html_file))
        .arg("-e")
        .arg(format!("TASK_REPLY_ATTACHMENTS_DIR={}", attachments_dir))
        .arg("-e")
        .arg(format!("TASK_METADATA_FILE={}", metadata_file))
        .arg("-e")
        .arg(format!("TASK_ROOT_DIR={}", container_root));
}

fn append_passthrough_env(command: &mut Command, names: &[String]) {
    for name in names {
        if let Some(value) = env::var_os(name) {
            command.arg("-e").arg(format!("{}={}", name, value.to_string_lossy()));
        }
    }
}

fn append_secret_env(command: &mut Command, path: &Path) -> Result<()> {
    for (key, value) in read_env_pairs(path)? {
        command.arg("-e").arg(format!("{}={}", key, value));
    }
    Ok(())
}

fn read_env_pairs(path: &Path) -> Result<Vec<(String, String)>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

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

fn pool_name(prefix: &str, host_root: &Path, image: &str) -> String {
    let mut hasher = DefaultHasher::new();
    host_root.hash(&mut hasher);
    image.hash(&mut hasher);
    format!("{}-{:x}", sanitize_name(prefix), hasher.finish())
}

fn sanitize_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_lowercase();

    if sanitized.is_empty() {
        "dowhiz-codex-pool".to_string()
    } else {
        sanitized
    }
}

fn normalize_container_root(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "/srv/dowhiz/tasks".to_string()
    } else {
        trimmed.trim_end_matches('/').to_string()
    }
}

fn join_container_path(root: &str, relative: &Path) -> String {
    let relative = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    format!("{}/{}", root.trim_end_matches('/'), relative)
}

fn canonicalize(path: &PathBuf) -> Result<PathBuf> {
    if path.exists() {
        Ok(path.canonicalize()?)
    } else {
        Err(anyhow!("workspace does not exist: {}", path.display()))
    }
}

fn ensure_success(success: bool, stderr: &[u8], operation: &str) -> Result<()> {
    if success {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(stderr);
        bail!("{} failed: {}", operation, stderr.trim());
    }
}
