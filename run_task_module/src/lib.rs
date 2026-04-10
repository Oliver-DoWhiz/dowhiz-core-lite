mod container;
mod local;
mod prompt;
mod types;
mod workspace;

pub use types::{ContainerMode, RunTaskOutput, RunTaskParams};

use anyhow::Result;

pub fn run_task(params: &RunTaskParams) -> Result<RunTaskOutput> {
    let prepared = workspace::prepare_workspace(&params.workspace_dir, &params.prompt)?;
    let stdout = if params.use_container {
        container::run_in_container(&prepared, params)?
    } else {
        local::run_locally(&prepared)?
    };

    workspace::write_reply_html(&prepared.reply_html_path, &stdout)?;

    Ok(RunTaskOutput {
        reply_html_path: prepared.reply_html_path,
        reply_attachments_dir: prepared.reply_attachments_dir,
        stdout,
    })
}
