use scheduler_module::config::GatewayConfig;
use scheduler_module::service::run_gateway;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt().with_target(false).init();
    run_gateway(GatewayConfig::from_env()).await
}
