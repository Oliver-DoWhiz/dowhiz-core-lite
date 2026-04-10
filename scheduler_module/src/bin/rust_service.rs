use scheduler_module::config::WorkerConfig;
use scheduler_module::worker::WorkerService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    WorkerService::new(WorkerConfig::from_env())?
        .run_forever()
        .await
}
