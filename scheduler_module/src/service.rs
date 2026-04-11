use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Multipart, Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use crate::account_registry::{AccountRegistry, AccountRegistryError};
use crate::attachment_store::AttachmentUploadStore;
use crate::config::GatewayConfig;
use crate::inbound_email::{
    persist_postmark_inbound_artifacts, task_request_from_postmark, PostmarkInboundPayload,
};
use crate::models::{CreateTaskRequest, QueuedTask, UploadAttachmentsResponse};
use crate::queue::FileQueue;
use crate::scheduler::TaskScheduler;
use crate::task_inspector::{TaskInspector, TaskSnapshot};

#[derive(Clone)]
struct AppState {
    scheduler: Arc<TaskScheduler>,
    inspector: Arc<TaskInspector>,
    account_registry: Arc<AccountRegistry>,
    attachment_store: Arc<AttachmentUploadStore>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct AccountIdSuggestionResponse {
    account_id: String,
}

pub async fn run_gateway(config: GatewayConfig) -> Result<()> {
    let queue = FileQueue::new(config.queue_root.clone())?;
    let scheduler = Arc::new(TaskScheduler::new(queue, config.tasks_root.clone())?);
    let inspector = Arc::new(TaskInspector::new(config.queue_root.clone()));
    let account_registry = Arc::new(AccountRegistry::load(config.account_registry_path)?);
    let attachment_store = Arc::new(AttachmentUploadStore::new(
        config.attachment_upload_root,
    )?);
    let state = AppState {
        scheduler,
        inspector,
        account_registry,
        attachment_store,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/account-ids/suggest", post(generate_account_id))
        .route("/uploads", post(upload_attachments))
        .route("/tasks", post(create_task))
        .route("/tasks/:task_id", get(get_task))
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

async fn generate_account_id(
    State(state): State<AppState>,
) -> std::result::Result<Json<AccountIdSuggestionResponse>, (StatusCode, String)> {
    let account_id = state
        .account_registry
        .generate_available_account_id()
        .map_err(account_registry_error)?;
    Ok(Json(AccountIdSuggestionResponse { account_id }))
}

async fn create_task(
    State(state): State<AppState>,
    Json(request): Json<CreateTaskRequest>,
) -> std::result::Result<Json<QueuedTask>, (StatusCode, String)> {
    let attachment_count = request.attachment_refs.len();
    let attachment_refs = request.attachment_refs.clone();
    let (normalized_request, resolved_account) =
        state.account_registry.resolve_create_request(request).map_err(account_registry_error)?;

    tracing::info!(
        channel = %normalized_request.channel,
        customer_email = %normalized_request.customer_email,
        subject = %normalized_request.subject,
        attachment_count,
        "received direct task submission"
    );
    let account_registry = state.account_registry.clone();
    let attachment_store = state.attachment_store.clone();
    let queued = state
        .scheduler
        .submit_with_initializer(normalized_request, move |workspace_dir| {
            account_registry.materialize_memory(workspace_dir, &resolved_account)?;
            attachment_store.materialize_refs(workspace_dir, &attachment_refs)
        })
        .map_err(internal_error)?;
    tracing::info!(
        task_id = %queued.id,
        workspace_key = %queued.workspace_key,
        workspace_dir = %queued.workspace_dir,
        "queued direct task submission"
    );
    Ok(Json(queued))
}

async fn upload_attachments(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> std::result::Result<Json<UploadAttachmentsResponse>, (StatusCode, String)> {
    let mut attachments = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|err| internal_error(anyhow::Error::new(err)))?
    {
        let file_name = field.file_name().unwrap_or("").to_string();
        if file_name.trim().is_empty() {
            continue;
        }

        let content_type = field
            .content_type()
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = field
            .bytes()
            .await
            .map_err(|err| internal_error(anyhow::Error::new(err)))?;
        let uploaded = state
            .attachment_store
            .stage_bytes(&file_name, &content_type, bytes.as_ref())
            .map_err(internal_error)?;
        attachments.push(uploaded);
    }

    if attachments.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "expected at least one uploaded attachment".to_string(),
        ));
    }

    tracing::info!(attachment_count = attachments.len(), "staged local attachments");
    Ok(Json(UploadAttachmentsResponse { attachments }))
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
    tracing::info!(
        from = %payload.from,
        subject = %payload.subject,
        attachment_count = payload.attachments.len(),
        message_id = %payload.message_id,
        "received inbound Postmark webhook"
    );
    let (request, resolved_account) = state
        .account_registry
        .resolve_inbound_request(task_request_from_postmark(&payload))
        .map_err(account_registry_error)?;
    tracing::debug!(
        customer_email = %request.customer_email,
        reply_to = %request.reply_to,
        channel = %request.channel,
        message_id = %payload.message_id,
        "normalized inbound Postmark payload into task request"
    );
    let queued = state
        .scheduler
        .submit_with_initializer(request.clone(), |workspace_dir| {
            state
                .account_registry
                .materialize_memory(workspace_dir, &resolved_account)?;
            persist_postmark_inbound_artifacts(workspace_dir, &payload, &request)
        })
        .map_err(internal_error)?;
    tracing::info!(
        task_id = %queued.id,
        workspace_key = %queued.workspace_key,
        workspace_dir = %queued.workspace_dir,
        message_id = %payload.message_id,
        "queued inbound Postmark task"
    );

    Ok(Json(queued))
}

fn internal_error(err: anyhow::Error) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn account_registry_error(err: AccountRegistryError) -> (StatusCode, String) {
    match err {
        AccountRegistryError::AccountIdTaken(message) => {
            (StatusCode::CONFLICT, format!("account_id '{}' has already been taken", message))
        }
        AccountRegistryError::EmailAlreadyBound { email, account_id } => (
            StatusCode::CONFLICT,
            format!("email '{}' is already linked to account '{}'", email, account_id),
        ),
        AccountRegistryError::InvalidAccountId(account_id) => (
            StatusCode::BAD_REQUEST,
            format!(
                "account_id '{}' must use only letters, numbers, hyphens, or underscores",
                account_id
            ),
        ),
        AccountRegistryError::Storage(err) => internal_error(err),
    }
}
