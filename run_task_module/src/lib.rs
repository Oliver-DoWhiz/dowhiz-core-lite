mod container;
mod local;
mod prompt;
mod types;
mod workspace;

pub use types::{ContainerMode, RunTaskOutput, RunTaskParams};

use anyhow::Result;

pub fn run_task(params: &RunTaskParams) -> Result<RunTaskOutput> {
    tracing::info!(
        workspace_dir = %params.workspace_dir.display(),
        use_container = params.use_container,
        "preparing task workspace for execution"
    );
    let prepared = workspace::prepare_workspace(&params.workspace_dir, &params.prompt)?;
    tracing::debug!(
        workspace_dir = %prepared.workspace_dir.display(),
        prompt_path = %prepared.prompt_path.display(),
        stdout_path = %prepared.stdout_path.display(),
        manifest_path = %prepared.manifest_path.display(),
        "prepared task workspace artifacts"
    );
    let stdout = if params.use_container {
        tracing::info!(
            workspace_dir = %prepared.workspace_dir.display(),
            container_image = %params.container_image,
            "running task inside container boundary"
        );
        container::run_in_container(&prepared, params)?
    } else {
        tracing::info!(
            workspace_dir = %prepared.workspace_dir.display(),
            "running task on local host"
        );
        local::run_locally(&prepared)?
    };

    workspace::write_reply_html(&prepared.reply_html_path, &stdout)?;
    tracing::info!(
        workspace_dir = %prepared.workspace_dir.display(),
        reply_html_path = %prepared.reply_html_path.display(),
        stdout_bytes = stdout.len(),
        "wrote reply HTML artifact"
    );

    Ok(RunTaskOutput {
        reply_html_path: prepared.reply_html_path,
        reply_attachments_dir: prepared.reply_attachments_dir,
        stdout,
    })
}
