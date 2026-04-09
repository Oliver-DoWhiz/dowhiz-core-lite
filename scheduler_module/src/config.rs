use std::env;
use std::path::PathBuf;
use std::time::Duration;

use run_task_module::ContainerMode;

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
    pub queue_root: PathBuf,
    pub tasks_root: PathBuf,
}

impl GatewayConfig {
    pub fn from_env() -> Self {
        Self {
            host: env::var("GATEWAY_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(9100),
            queue_root: env_path("QUEUE_ROOT", ".workspace/queue"),
            tasks_root: env_path("TASKS_ROOT", ".workspace/tasks"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub queue_root: PathBuf,
    pub tasks_root: PathBuf,
    pub poll_interval: Duration,
    pub use_container: bool,
    pub container_image: String,
    pub container_mode: ContainerMode,
    pub container_host_root: Option<PathBuf>,
    pub container_workspace_root: String,
    pub container_pool_name: String,
    pub container_env_passthrough: Vec<String>,
    pub outbound_mode: OutboundMode,
    pub postmark_api_base_url: String,
    pub postmark_server_token: Option<String>,
    pub postmark_from: Option<String>,
    pub postmark_message_stream: Option<String>,
    pub postmark_tag: Option<String>,
}

impl WorkerConfig {
    pub fn from_env() -> Self {
        Self {
            queue_root: env_path("QUEUE_ROOT", ".workspace/queue"),
            tasks_root: env_path("TASKS_ROOT", ".workspace/tasks"),
            poll_interval: Duration::from_millis(
                env::var("WORKER_POLL_MS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .unwrap_or(1_000),
            ),
            use_container: env::var("RUN_TASK_USE_CONTAINER")
                .ok()
                .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
                .unwrap_or(false),
            container_image: env::var("RUN_TASK_CONTAINER_IMAGE")
                .unwrap_or_else(|_| "dowhiz/codex-runner:latest".to_string()),
            container_mode: match env::var("RUN_TASK_CONTAINER_MODE")
                .unwrap_or_else(|_| "one_shot".to_string())
                .as_str()
            {
                "warm_pool" => ContainerMode::WarmPool,
                _ => ContainerMode::OneShot,
            },
            container_host_root: env::var("RUN_TASK_CONTAINER_HOST_ROOT")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            container_workspace_root: env::var("RUN_TASK_CONTAINER_WORKSPACE_ROOT")
                .unwrap_or_else(|_| "/srv/dowhiz/tasks".to_string()),
            container_pool_name: env::var("RUN_TASK_CONTAINER_POOL_NAME")
                .unwrap_or_else(|_| "dowhiz-codex-pool".to_string()),
            container_env_passthrough: env_csv("RUN_TASK_CONTAINER_ENV_PASSTHROUGH"),
            outbound_mode: match env::var("OUTBOUND_DELIVERY_MODE")
                .unwrap_or_else(|_| "preview".to_string())
                .as_str()
            {
                "postmark" => OutboundMode::Postmark,
                _ => OutboundMode::PreviewOnly,
            },
            postmark_api_base_url: env::var("POSTMARK_API_BASE_URL")
                .unwrap_or_else(|_| "https://api.postmarkapp.com".to_string()),
            postmark_server_token: env::var("POSTMARK_SERVER_TOKEN")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            postmark_from: env::var("POSTMARK_FROM")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            postmark_message_stream: env::var("POSTMARK_MESSAGE_STREAM")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            postmark_tag: env::var("POSTMARK_TAG")
                .ok()
                .filter(|value| !value.trim().is_empty()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundMode {
    PreviewOnly,
    Postmark,
}

fn env_path(key: &str, fallback: &str) -> PathBuf {
    env::var(key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(fallback))
}

fn env_csv(key: &str) -> Vec<String> {
    env::var(key)
        .ok()
        .map(|value| {
            value
                .split(',')
                .filter_map(|item| {
                    let trimmed = item.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}
