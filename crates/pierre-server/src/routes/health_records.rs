// ABOUTME: The athlete's own health records: the served list, the review queue, confirm and reject, lab report upload and delete
// ABOUTME: First-party only: an outside credential (MCP, A2A, an API key) is refused, since T2 records never reach one (carnet#1544)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Health records routes
//!
//! Owl phase 5, slice 2. The athlete reads everything they hold here, the
//! resource as read included; who else reads it is decided by the tier
//! consent in the reader every other surface uses
//! ([`pierre_services::health_records::read`]).
//!
//! - `GET /api/me/health-records?kind=labs|clinical|glucose|self_reports&since&until&limit`
//!   — the served records of one kind (read as they are, or confirmed);
//! - `GET /api/me/health-records/review` — the records waiting for the
//!   athlete, undecided (only lab reports produce them);
//! - `POST /api/me/health-records/{id}/confirm`, `…/reject` — the athlete's
//!   answer to a pending record; `404` for any other record;
//! - `POST /api/me/lab-documents` — a lab report's PDF as the raw body
//!   (`201` when stored, `200` when the athlete already held the file);
//! - `GET /api/me/lab-documents`, `DELETE /api/me/lab-documents/{id}`.
//!
//! None of them serves an outside credential: a T2 record never reaches an
//! outside client until grants carry authorization details (slice 3).

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    ClinicalSelection, RecordWindow, ReviewDecision, MAX_HEALTH_RECORDS_PAGE,
};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::{resolve_tenant_in, tenant::require, TenantMode};
use pierre_services::health_records::read::{
    HealthRecordsPage, RecordKind, RecordView, RecordsRead,
};
use pierre_services::stored_health::as_stored;
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;
use crate::routes::own_screens::served_to_outside_client;
use crate::services::lab_documents::{
    delete_lab_document, list_lab_documents, upload_lab_document, LabDocumentView, LabRecordView,
    MAX_LAB_DOCUMENT_BYTES,
};

/// The page a list returns when none is asked for.
const DEFAULT_PAGE: u32 = 100;

/// The query of `GET /api/me/health-records`.
#[derive(Debug, Deserialize)]
pub struct RecordsQuery {
    /// `labs`, `clinical`, `glucose` or `self_reports`.
    pub kind: String,
    /// From this instant, inclusive.
    pub since: Option<Timestamp>,
    /// Until this instant, exclusive.
    pub until: Option<Timestamp>,
    /// At most this many records, 1 to 200.
    pub limit: Option<u32>,
}

/// The body of `GET /api/me/health-records/review`.
#[derive(Debug, Serialize)]
pub struct ReviewQueue {
    /// The records waiting for the athlete, newest first.
    pub pending: Vec<LabRecordView>,
}

/// The body of `GET /api/me/lab-documents`.
#[derive(Debug, Serialize)]
pub struct LabDocumentList {
    /// The athlete's lab documents, newest first.
    pub documents: Vec<LabDocumentView>,
}

/// The health records routes.
pub fn health_records_routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/me/health-records", get(get_records))
        .route("/api/me/health-records/review", get(get_review))
        .route("/api/me/health-records/{id}/confirm", post(confirm_record))
        .route("/api/me/health-records/{id}/reject", post(reject_record))
        .route(
            "/api/me/lab-documents",
            post(post_lab_document)
                .layer(DefaultBodyLimit::max(MAX_LAB_DOCUMENT_BYTES))
                .get(get_lab_documents),
        )
        .route("/api/me/lab-documents/{id}", delete(delete_document))
}

/// The caller's tenant, refusing an outside credential: health records are
/// read on the athlete's own screens only.
async fn own_tenant(resources: &ServerContext, auth: &AuthenticatedUser) -> AppResult<TenantId> {
    if served_to_outside_client(auth) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Health records are not served to outside apps. The athlete sees them in Dravr.",
        ));
    }
    let tenants = resources.common.repos.tenants.as_ref();
    require(resolve_tenant_in(tenants, auth, TenantMode::Required).await?)
}

async fn get_records(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<RecordsQuery>,
) -> AppResult<Json<HealthRecordsPage>> {
    let tenant_id = own_tenant(&resources, &auth).await?;
    let kind = RecordKind::parse(&query.kind).ok_or_else(|| {
        AppError::invalid_input("kind must be labs, clinical, glucose or self_reports")
    })?;
    let window = RecordWindow {
        since: query.since,
        until: query.until,
        limit: query
            .limit
            .unwrap_or(DEFAULT_PAGE)
            .clamp(1, MAX_HEALTH_RECORDS_PAGE),
    };
    let read = RecordsRead {
        repos: &resources.common.repos,
        terms: resources.fitness.provider_registry.as_ref(),
        user_id: auth.user_id,
        tenant: &tenant_id,
        view: RecordView::Owner,
        coach: None,
        project: as_stored,
    };
    Ok(Json(read.read(kind, window).await?))
}

async fn get_review(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<ReviewQueue>> {
    let tenant_id = own_tenant(&resources, &auth).await?;
    let pending = resources
        .common
        .repos
        .health_records
        .list_clinical_records(
            &tenant_id,
            auth.user_id,
            ClinicalSelection::Pending,
            RecordWindow {
                since: None,
                until: None,
                limit: MAX_HEALTH_RECORDS_PAGE,
            },
        )
        .await?;
    Ok(Json(ReviewQueue {
        pending: pending.into_iter().map(LabRecordView::from).collect(),
    }))
}

/// Record the athlete's `decision` on their pending record `id`.
async fn decide(
    resources: &ServerContext,
    auth: &AuthenticatedUser,
    id: &str,
    decision: ReviewDecision,
) -> AppResult<StatusCode> {
    let tenant_id = own_tenant(resources, auth).await?;
    let decided = resources
        .common
        .repos
        .health_records
        .decide_clinical_record(&tenant_id, auth.user_id, id, decision, Timestamp::now())
        .await?;
    if decided {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found(format!("pending health record {id}")))
    }
}

async fn confirm_record(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&resources, &auth, &id, ReviewDecision::Confirmed).await
}

async fn reject_record(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    decide(&resources, &auth, &id, ReviewDecision::Rejected).await
}

async fn post_lab_document(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    body: Bytes,
) -> AppResult<(StatusCode, Json<LabDocumentView>)> {
    let tenant_id = own_tenant(&resources, &auth).await?;
    let view = upload_lab_document(&resources, tenant_id, auth.user_id, body).await?;
    let status = if view.new {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(view)))
}

async fn get_lab_documents(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<LabDocumentList>> {
    let tenant_id = own_tenant(&resources, &auth).await?;
    Ok(Json(LabDocumentList {
        documents: list_lab_documents(&resources, &tenant_id, auth.user_id).await?,
    }))
}

async fn delete_document(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    let tenant_id = own_tenant(&resources, &auth).await?;
    if delete_lab_document(&resources, &tenant_id, auth.user_id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AppError::not_found(format!("lab document {id}")))
    }
}
