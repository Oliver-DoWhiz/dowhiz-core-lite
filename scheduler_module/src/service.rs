use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::config::GatewayConfig;
use crate::inbound_email::{
    persist_postmark_inbound_artifacts, task_request_from_postmark, PostmarkInboundPayload,
};
use crate::models::{InboundTaskRequest, QueuedTask};
use crate::queue::FileQueue;
use crate::scheduler::TaskScheduler;
use crate::task_inspector::{TaskInspector, TaskSnapshot};

#[derive(Clone)]
struct AppState {
    scheduler: Arc<TaskScheduler>,
    inspector: Arc<TaskInspector>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub async fn run_gateway(config: GatewayConfig) -> Result<()> {
    let queue = FileQueue::new(config.queue_root.clone())?;
    let scheduler = Arc::new(TaskScheduler::new(queue, config.tasks_root.clone())?);
    let inspector = Arc::new(TaskInspector::new(config.queue_root.clone()));
    let state = AppState {
        scheduler,
        inspector,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/tasks", post(create_task))
        .route("/tasks/{task_id}", get(get_task))
        .route("/webhooks/postmark/inbound", post(receive_postmark_inbound))
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
) -> std::result::Result<Json<QueuedTask>, (StatusCode, String)> {
    state
        .scheduler
        .submit(request)
        .map(Json)
        .map_err(internal_error)
}

async fn get_task(
    Path(task_id): Path<String>,
    State(state): State<AppState>,
) -> std::result::Result<Json<TaskSnapshot>, (StatusCode, String)> {
    match state.inspector.get(&task_id).map_err(internal_error)? {
        Some(snapshot) => Ok(Json(snapshot)),
        None => Err((StatusCode::NOT_FOUND, format!("task not found: {}", task_id))),
    }
}

async fn receive_postmark_inbound(
    State(state): State<AppState>,
    Json(payload): Json<PostmarkInboundPayload>,
) -> std::result::Result<Json<QueuedTask>, (StatusCode, String)> {
    let request = task_request_from_postmark(&payload);
    let queued = state
        .scheduler
        .submit_with_initializer(request.clone(), |workspace_dir| {
            persist_postmark_inbound_artifacts(workspace_dir, &payload, &request)
        })
        .map_err(internal_error)?;

    Ok(Json(queued))
}

fn internal_error(err: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}
