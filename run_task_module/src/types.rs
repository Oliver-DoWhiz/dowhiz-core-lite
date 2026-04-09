use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RunTaskParams {
    pub workspace_dir: PathBuf,
    pub prompt: String,
    pub use_container: bool,
    pub container_image: String,
}

#[derive(Debug, Clone)]
pub struct RunTaskOutput {
    pub reply_html_path: PathBuf,
    pub reply_attachments_dir: PathBuf,
    pub stdout: String,
}

#[derive(Debug, Clone)]
pub struct PreparedWorkspace {
    pub workspace_dir: PathBuf,
    pub prompt_path: PathBuf,
    pub stdout_path: PathBuf,
    pub reply_html_path: PathBuf,
    pub reply_attachments_dir: PathBuf,
    pub prompt: String,
}
