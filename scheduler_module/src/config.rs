use std::env;
use std::path::PathBuf;
use std::time::Duration;

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
        }
    }
}

fn env_path(key: &str, fallback: &str) -> PathBuf {
    env::var(key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(fallback))
}
