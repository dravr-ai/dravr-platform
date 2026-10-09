// ABOUTME: POST /api/me/activities/upload keeps a completed workout's .fit as the athlete's activities
// ABOUTME: DELETE /api/me/activities/upload/{id} removes one of them, and the file with its last session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The upload of a completed workout's `.fit` file (carnet#818).
//!
//! The body is the file itself (`application/octet-stream`), so no form
//! encoding stands between the client's file picker and the decoder. A body
//! over [`MAX_UPLOAD_BYTES`] is refused before it is read (`413`); a file
//! that is not a completed activity answers `400`, and one whose every
//! session the athlete already holds answers `409`. See
//! [`crate::services::activity_upload`] for what is kept.
//!
//! `DELETE /api/me/activities/{provider}/{id}`, on the route the activity
//! view reads, deletes one uploaded activity of the caller's (`204`). Only an
//! upload can be deleted here: a provider's activity is deleted on the
//! provider, and its sync removes it. Any other provider, or an id the caller
//! did not upload in this tenant (another athlete's), answers `404`.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::CachedActivityRow;
use pierre_middleware::extractors::AuthenticatedUser;
use serde::Serialize;

use super::{active_tenant, HomeActivity};
use crate::mcp::resources::ServerContext;
use crate::services::activity_upload::{
    delete_uploaded_activity, upload_activity_file, HeldCopy, MAX_UPLOAD_BYTES,
};
use crate::tools::runtime_adapter::into_runtime;

/// A copy of one of the file's sessions the athlete already held.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HeldActivity {
    /// The provider the copy is from, by its user-facing slug (`upload` for
    /// an earlier upload).
    pub provider: String,
    /// That provider's id for it.
    pub id: String,
}

impl From<HeldCopy> for HeldActivity {
    fn from(copy: HeldCopy) -> Self {
        Self {
            provider: copy.provider,
            id: copy.id,
        }
    }
}

/// Body of a `201` from `POST /api/me/activities/upload`.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityUploadResponse {
    /// The Home rows the file's new sessions became, in file order.
    pub activities: Vec<HomeActivity>,
    /// The copies already held of the file's other sessions; empty for a
    /// single-sport file.
    pub already_held: Vec<HeldActivity>,
}

/// The upload route, with the body limit that bounds it.
pub(super) fn upload_routes() -> Router<Arc<ServerContext>> {
    Router::new().route(
        "/api/me/activities/upload",
        post(upload_activity).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
    )
}

/// `DELETE /api/me/activities/{provider}/{activity_id}`: delete one of the
/// caller's uploaded activities. Any provider but [`UPLOAD`] is not found.
pub(super) async fn delete_activity(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path((provider, activity_id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    if provider != UPLOAD {
        return Err(AppError::not_found(format!(
            "uploaded activity {provider}/{activity_id}"
        )));
    }
    let tenant_id = active_tenant(&auth)?;
    delete_uploaded_activity(
        &into_runtime(&resources),
        tenant_id,
        auth.user_id,
        &activity_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn upload_activity(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    body: Bytes,
) -> AppResult<(StatusCode, Json<ActivityUploadResponse>)> {
    let tenant_id = active_tenant(&auth)?;
    let runtime = into_runtime(&resources);
    let outcome = upload_activity_file(&runtime, tenant_id, auth.user_id, &body).await?;
    let activities = outcome
        .stored
        .into_iter()
        .map(|activity| {
            let row = CachedActivityRow {
                provider: UPLOAD.to_owned(),
                activity,
                route: None,
                detail_read: false,
            };
            HomeActivity::from_session(&row, &row.activity)
        })
        .collect();
    Ok((
        StatusCode::CREATED,
        Json(ActivityUploadResponse {
            activities,
            already_held: outcome
                .already_held
                .into_iter()
                .map(HeldActivity::from)
                .collect(),
        }),
    ))
}
