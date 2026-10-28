// ABOUTME: /api/imports — the athlete imports their own history from a service's export: check, start, follow, cancel, delete
// ABOUTME: Session only (no API key, MCP token or delegated grant); the hosted import page's start/follow/cancel take its import link-token

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Import routes (carnet#1218)
//!
//! | Route | Answers |
//! |---|---|
//! | `GET /api/imports` | whether imports run here, the offered sources, and each source's running or latest import |
//! | `POST /api/imports/{source}/check` `{url}` | whether an import can start from the link (`200`, with a refusal code when not) |
//! | `POST /api/imports/{source}` `{url?}` | `201`: an import waiting for its link, or one started from it; `409` while another runs |
//! | `GET /api/imports/{source}/current` | the source's running or latest import, or `null` |
//! | `GET /api/imports/{source}/{id}/report?offset&limit` | a page of the rows it did not import |
//! | `POST /api/imports/{source}/{id}/cancel` | whether it was still running |
//! | `DELETE /api/imports/{source}/data` | everything imported from the source deleted |
//!
//! Every route takes the athlete's own session, in their active tenant: an
//! API key, a personal MCP token or a delegated OAuth grant is refused, so no
//! agent, MCP client or A2A caller can start, cancel or delete an import.
//! `source` is `strava_archive`; any other is `404`.
//!
//! `POST /internal/imports/{id}/step` exists only where Cloud Tasks delivers
//! the steps; its one gate is the delivery's OIDC token.
//!
//! ## The hosted import page (carnet#1221)
//!
//! The page a chat's connect picker opens (`/providers/connect/import`)
//! calls three routes of its own, each with the *import link-token* the page
//! was rendered with (`Authorization: Bearer`, minted by
//! [`pierre_middleware::provider_link_token::mint_import_link_token`]):
//!
//! | Route | Answers |
//! |---|---|
//! | `GET /providers/connect/import/{source}/current` | whether imports run here, and the source's running or latest import |
//! | `POST /providers/connect/import/{source}` `{url?}` | as `POST /api/imports/{source}` |
//! | `POST /providers/connect/import/{source}/{id}/cancel` | as `POST /api/imports/{source}/{id}/cancel` |
//!
//! The token starts, follows and cancels; it never reads an import's report
//! or deletes the imported history, and every `/api/imports` route refuses
//! it. It is valid for one source and acts in the tenant and account it
//! names, which must still be a member of that tenant.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use dravr_barrage::store::PageReq;
use dravr_barrage::{Scope, SourceKey};
use pierre_auth::auth::AuthMethod;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_middleware::provider_link_token;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::archive_import::ops::{
    self, ImportView, LinkCheckView, ReportPage, OFFERED_SOURCES,
};
use crate::services::archive_import::runner::StepTask;

/// The most report rows one page returns.
const MAX_REPORT_PAGE: u32 = 200;

/// The athlete's routes.
pub fn import_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/imports", get(list_imports))
        .route("/api/imports/{source}", post(start_import))
        .route("/api/imports/{source}/check", post(check_link))
        .route("/api/imports/{source}/current", get(current_import))
        .route("/api/imports/{source}/data", delete(delete_history))
        .route("/api/imports/{source}/{id}/report", get(import_report))
        .route("/api/imports/{source}/{id}/cancel", post(cancel_import))
        .route(
            "/providers/connect/import/{source}",
            post(link_start_import),
        )
        .route(
            "/providers/connect/import/{source}/current",
            get(link_current_import),
        )
        .route(
            "/providers/connect/import/{source}/{id}/cancel",
            post(link_cancel_import),
        )
}

/// The step route, mounted only where Cloud Tasks delivers import steps.
pub fn internal_import_routes(resources: &ServerContext) -> Router<Arc<ServerContext>> {
    let router = Router::new();
    if resources
        .common
        .archive_imports
        .wiring()
        .is_ok_and(|wiring| wiring.runner.cloud_tasks().is_some())
    {
        router.route("/internal/imports/{id}/step", post(run_step))
    } else {
        router
    }
}

/// The caller's scope, refusing every credential but the athlete's own
/// session.
fn session_scope(auth: &AuthenticatedUser) -> AppResult<Scope> {
    if !matches!(auth.auth_method, AuthMethod::JwtToken { .. }) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Imports are started and managed only from Dravr's own apps",
        ));
    }
    let tenant = auth
        .active_tenant_id
        .map(TenantId::from_uuid)
        .ok_or_else(|| AppError::auth_invalid("Imports require an active tenant in the JWT"))?;
    Ok(ops::scope_of(tenant, auth.user_id))
}

/// The scope and source an import link-token names, for the hosted import
/// page's routes.
///
/// The token must be an import token for `source` itself, and the account it
/// names must still belong to its tenant: an account removed from the tenant
/// since the link was sent starts nothing there.
async fn link_scope(
    resources: &ServerContext,
    headers: &HeaderMap,
    source: &str,
) -> AppResult<(Scope, SourceKey)> {
    let source = ops::offered_source(source)?;
    let token = provider_link_token::extract_bearer_link_token(
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok()),
    )
    .ok_or_else(AppError::auth_required)?;
    let claims = provider_link_token::verify_import_link_token(
        token,
        &resources.auth.admin_jwt_secret,
        source.as_str(),
    )?;
    let (Ok(user_id), Ok(tenant_id)) = (Uuid::parse_str(&claims.sub), Uuid::parse_str(&claims.tid))
    else {
        return Err(AppError::auth_invalid(
            "Import link-token has malformed identity claims",
        ));
    };
    let tenant_id = TenantId::from_uuid(tenant_id);
    if resources
        .common
        .repos
        .tenants
        .get_user_role(user_id, tenant_id)
        .await?
        .is_none()
    {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "The import link names an account outside its tenant",
        ));
    }
    Ok((ops::scope_of(tenant_id, user_id), source))
}

/// Body of `GET /providers/connect/import/{source}/current`.
#[derive(Debug, Serialize)]
struct HostedImportState {
    /// Whether imports run on this deployment.
    enabled: bool,
    /// The source's running or latest import.
    import: Option<ImportView>,
}

async fn link_current_import(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(source): Path<String>,
) -> AppResult<Json<HostedImportState>> {
    let (scope, source) = link_scope(&resources, &headers, &source).await?;
    if !resources.common.archive_imports.is_enabled() {
        return Ok(Json(HostedImportState {
            enabled: false,
            import: None,
        }));
    }
    Ok(Json(HostedImportState {
        enabled: true,
        import: ops::latest(&resources, &scope, source).await?,
    }))
}

async fn link_start_import(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(source): Path<String>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<ImportView>)> {
    let (scope, source) = link_scope(&resources, &headers, &source).await?;
    let body = start_body(&body)?;
    let view = ops::start(&resources, &scope, source, body.url.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn link_cancel_import(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path((source, id)): Path<(String, Uuid)>,
) -> AppResult<Json<Value>> {
    let (scope, source) = link_scope(&resources, &headers, &source).await?;
    let cancelled = ops::cancel(&resources, &scope, source, id).await?;
    Ok(Json(json!({ "cancelled": cancelled })))
}

/// Body of `GET /api/imports`.
#[derive(Debug, Serialize)]
struct ImportsOverview {
    /// Whether imports run on this deployment.
    enabled: bool,
    /// The sources an athlete may import.
    sources: Vec<&'static str>,
    /// Each source's running or latest import.
    imports: Vec<ImportView>,
}

async fn list_imports(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<ImportsOverview>> {
    let scope = session_scope(&auth)?;
    let sources = OFFERED_SOURCES
        .iter()
        .map(|source| source.as_str())
        .collect();
    if !resources.common.archive_imports.is_enabled() {
        return Ok(Json(ImportsOverview {
            enabled: false,
            sources,
            imports: Vec::new(),
        }));
    }
    Ok(Json(ImportsOverview {
        enabled: true,
        sources,
        imports: ops::latest_per_source(&resources, &scope).await?,
    }))
}

/// A link the athlete pasted.
#[derive(Debug, Deserialize)]
struct LinkBody {
    url: String,
}

async fn check_link(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
    Json(body): Json<LinkBody>,
) -> AppResult<Json<LinkCheckView>> {
    session_scope(&auth)?;
    ops::offered_source(&source)?;
    Ok(Json(ops::check_link(&resources, &body.url).await?))
}

/// Body of `POST /api/imports/{source}`: the link, or nothing to wait for it.
#[derive(Debug, Default, Deserialize)]
struct StartBody {
    #[serde(default)]
    url: Option<String>,
}

/// A start's body; an empty one asks for an import that waits for its link.
fn start_body(body: &[u8]) -> AppResult<StartBody> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(StartBody::default());
    }
    serde_json::from_slice(body).map_err(|e| AppError::invalid_input(format!("import body: {e}")))
}

async fn start_import(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
    body: Bytes,
) -> AppResult<(StatusCode, Json<ImportView>)> {
    let scope = session_scope(&auth)?;
    let source = ops::offered_source(&source)?;
    let body = start_body(&body)?;
    let view = ops::start(&resources, &scope, source, body.url.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(view)))
}

async fn current_import(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
) -> AppResult<Json<Value>> {
    let scope = session_scope(&auth)?;
    let source = ops::offered_source(&source)?;
    let view = ops::latest(&resources, &scope, source).await?;
    Ok(Json(json!({ "import": view })))
}

/// A report page's position.
#[derive(Debug, Deserialize)]
struct PageQuery {
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    limit: Option<u32>,
}

async fn import_report(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((source, id)): Path<(String, Uuid)>,
    Query(page): Query<PageQuery>,
) -> AppResult<Json<ReportPage>> {
    let scope = session_scope(&auth)?;
    let source = ops::offered_source(&source)?;
    let page = PageReq {
        limit: page.limit.unwrap_or(50).min(MAX_REPORT_PAGE),
        offset: page.offset,
    };
    Ok(Json(
        ops::report(&resources, &scope, source, id, page).await?,
    ))
}

async fn cancel_import(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((source, id)): Path<(String, Uuid)>,
) -> AppResult<Json<Value>> {
    let scope = session_scope(&auth)?;
    let source = ops::offered_source(&source)?;
    let cancelled = ops::cancel(&resources, &scope, source, id).await?;
    Ok(Json(json!({ "cancelled": cancelled })))
}

async fn delete_history(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(source): Path<String>,
) -> AppResult<Json<Value>> {
    let scope = session_scope(&auth)?;
    let source: SourceKey = ops::offered_source(&source)?;
    let originals = ops::delete_history(&resources, &scope, source).await?;
    Ok(Json(
        json!({ "deleted": true, "originals_deleted": originals }),
    ))
}

/// One step delivered by Cloud Tasks.
///
/// `200` closes the task (the step ran, or found nothing to do); an error
/// asks Cloud Tasks to deliver it again. The body's tenant and account are
/// the only scope the step reads the import under: a body naming another
/// athlete's import finds nothing.
async fn run_step(
    State(resources): State<Arc<ServerContext>>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Json(task): Json<StepTask>,
) -> AppResult<StatusCode> {
    let wiring = resources.common.archive_imports.wiring()?;
    let Some(tasks) = wiring.runner.cloud_tasks() else {
        return Err(AppError::not_found("import step runner"));
    };
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            warn!(import = %id, "import step delivery carried no bearer token");
            AppError::auth_required()
        })?;
    tasks.verify_delivery(token).await?;
    if task.import_id != id {
        return Err(AppError::invalid_input("the step names another import"));
    }
    if let Some(next) = ops::run_step(&resources, &task).await? {
        wiring.runner.schedule(&resources, next).await?;
    }
    Ok(StatusCode::OK)
}
