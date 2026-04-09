use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RunTaskParams {
    pub workspace_dir: PathBuf,
    pub prompt: String,
    pub use_container: bool,
    pub container_image: String,
    pub container_mode: ContainerMode,
    pub container_host_root: Option<PathBuf>,
    pub container_workspace_root: String,
    pub container_pool_name: String,
    pub env_passthrough: Vec<String>,
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
    pub manifest_path: PathBuf,
    pub secrets_env_path: PathBuf,
    pub prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerMode {
    OneShot,
    WarmPool,
}
