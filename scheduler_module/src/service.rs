use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::config::GatewayConfig;
use crate::models::{InboundTaskRequest, QueuedTask};
use crate::queue::FileQueue;
use crate::scheduler::TaskScheduler;

#[derive(Clone)]
struct AppState {
    scheduler: Arc<TaskScheduler>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub async fn run_gateway(config: GatewayConfig) -> Result<()> {
    let queue = FileQueue::new(config.queue_root.clone())?;
    let scheduler = Arc::new(TaskScheduler::new(queue, config.tasks_root.clone())?);
    let state = AppState { scheduler };

    let app = Router::new()
        .route("/health", get(health))
        .route("/tasks", post(create_task))
        .with_state(state);

    // Axum serves HTTP on top of a TCP socket here. In production, HTTPS should
    // terminate at a reverse proxy or load balancer in front of this gateway.
    let listener = tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await?;
    let addr = listener.local_addr().unwrap_or(SocketAddr::from(([0, 0, 0, 0], config.port)));
    tracing::info!("http gateway listening on {}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<InboundTaskRequest>,
) -> std::result::Result<Json<QueuedTask>, (axum::http::StatusCode, String)> {
    state
        .scheduler
        .submit(request)
        .map(Json)
        .map_err(internal_error)
}

fn internal_error(err: anyhow::Error) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
