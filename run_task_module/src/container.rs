use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;

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
    let workspace = canonicalize_path(&prepared.workspace_dir)?;
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
    let workspace = canonicalize_path(&prepared.workspace_dir)?;
    let relative_workspace = relative_workspace_key(&prepared.manifest_path, &workspace)?;
    let pool_name = pool_name(
        &params.container_pool_name,
        &params.container_image,
        &params.container_workspace_root,
    );
    ensure_pool_container(&pool_name, params)?;

    let container_workspace_root = normalize_container_root(&params.container_workspace_root);
    let container_workspace = join_container_path(&container_workspace_root, &relative_workspace);

    sync_workspace_to_pool(&pool_name, &workspace, &container_workspace)?;

    let exec_output = run_pool_exec(
        &pool_name,
        &container_workspace,
        &container_workspace_root,
        prepared,
        params,
    );
    let sync_back_result = sync_workspace_from_pool(&pool_name, &container_workspace, &workspace);
    let cleanup_result = cleanup_container_workspace(&pool_name, &container_workspace);

    sync_back_result?;
    cleanup_result?;
    let exec_output = exec_output?;
    ensure_success(
        exec_output.status.success(),
        &exec_output.stderr,
        "warm container exec",
    )?;

    Ok(())
}

fn run_pool_exec(
    pool_name: &str,
    container_workspace: &str,
    container_workspace_root: &str,
    prepared: &PreparedWorkspace,
    params: &RunTaskParams,
) -> Result<std::process::Output> {
    let mut command = Command::new("docker");
    command.arg("exec");
    append_passthrough_env(&mut command, &params.env_passthrough);
    append_secret_env(&mut command, &prepared.secrets_env_path)?;
    append_task_env(&mut command, container_workspace, container_workspace_root);
    command.arg(pool_name).arg("/app/exec_codex.sh");
    Ok(command.output()?)
}

fn ensure_pool_container(pool_name: &str, params: &RunTaskParams) -> Result<()> {
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

    let output = Command::new("docker")
        .arg("run")
        .arg("-d")
        .arg("--rm")
        .arg("--name")
        .arg(pool_name)
        .arg("-e")
        .arg("CODEX_RUNNER_MODE=pool")
        .arg(&params.container_image)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool bootstrap",
    )?;

    let container_root = normalize_container_root(&params.container_workspace_root);
    ensure_container_dir(pool_name, &container_root)?;
    Ok(())
}

fn sync_workspace_to_pool(pool_name: &str, workspace: &Path, container_workspace: &str) -> Result<()> {
    let parent = Path::new(container_workspace)
        .parent()
        .ok_or_else(|| anyhow!("container workspace has no parent: {}", container_workspace))?;
    ensure_container_dir(pool_name, &parent.display().to_string())?;
    cleanup_container_workspace(pool_name, container_workspace)?;
    ensure_container_dir(pool_name, container_workspace)?;

    let destination = format!("{}:{}", pool_name, container_workspace);
    let source = format!("{}/.", workspace.display());
    let output = Command::new("docker")
        .arg("cp")
        .arg(source)
        .arg(destination)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool workspace upload",
    )?;
    Ok(())
}

fn sync_workspace_from_pool(pool_name: &str, container_workspace: &str, workspace: &Path) -> Result<()> {
    fs::create_dir_all(workspace)?;
    let source = format!("{}:{}/.", pool_name, container_workspace);
    let output = Command::new("docker")
        .arg("cp")
        .arg(source)
        .arg(workspace)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool workspace download",
    )?;
    Ok(())
}

fn ensure_container_dir(pool_name: &str, directory: &str) -> Result<()> {
    let output = Command::new("docker")
        .arg("exec")
        .arg(pool_name)
        .arg("mkdir")
        .arg("-p")
        .arg(directory)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool mkdir",
    )?;
    Ok(())
}

fn cleanup_container_workspace(pool_name: &str, container_workspace: &str) -> Result<()> {
    let output = Command::new("docker")
        .arg("exec")
        .arg(pool_name)
        .arg("rm")
        .arg("-rf")
        .arg(container_workspace)
        .output()?;
    ensure_success(
        output.status.success(),
        &output.stderr,
        "warm pool workspace cleanup",
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

fn relative_workspace_key(manifest_path: &Path, workspace: &Path) -> Result<PathBuf> {
    if manifest_path.exists() {
        let manifest: WorkspaceManifestRef = serde_json::from_str(&fs::read_to_string(manifest_path)?)?;
        if !manifest.workspace_key.trim().is_empty() {
            return Ok(PathBuf::from(manifest.workspace_key));
        }
    }

    let fallback = workspace
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("task-workspace");
    Ok(PathBuf::from(fallback))
}

fn pool_name(prefix: &str, image: &str, container_root: &str) -> String {
    let mut hasher = DefaultHasher::new();
    image.hash(&mut hasher);
    container_root.hash(&mut hasher);
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

fn canonicalize_path(path: &Path) -> Result<PathBuf> {
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

#[derive(Debug, Deserialize)]
struct WorkspaceManifestRef {
    workspace_key: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn uses_manifest_workspace_key_for_pool_partitioning() {
        let root = temp_dir("warm-pool-key");
        let manifest_path = root.join("workspace_manifest.json");
        fs::write(&manifest_path, r#"{"workspace_key":"prod/user_42/task-1"}"#).unwrap();

        let relative = relative_workspace_key(&manifest_path, &root).unwrap();

        assert_eq!(relative, PathBuf::from("prod/user_42/task-1"));
    }

    #[test]
    fn joins_container_paths_without_double_slashes() {
        let path = join_container_path("/srv/dowhiz/tasks/", Path::new("prod/user_42/task-1"));
        assert_eq!(path, "/srv/dowhiz/tasks/prod/user_42/task-1");
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
