// ABOUTME: The originals bundle routes: ask for one (optionally sealed with a passphrase), follow it, download it
// ABOUTME: Own first-party session only, metered 3 a day; a download is a 15-minute signed URL or the bytes streamed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/user/export/originals` (carnet#1541).
//!
//! `GET /api/user/export` leaves out the bytes of the files Dravr keeps for
//! the athlete; these routes build them into one bundle the export names in
//! its `originals` field:
//!
//! - `POST /api/user/export/originals` (`{"passphrase": "…"}` optional) asks
//!   for one and answers 202 with its view (409 while a build that cannot
//!   stand for it runs). A passphrase seals it
//!   (`pierre_services::originals_seal`); it is used once, off the async
//!   runtime, and never stored or logged. Each account may start
//!   [`MAX_BUNDLES_PER_WINDOW`] builds a day; a request answered by a build
//!   in flight, a fresh bundle or a busy refusal is not counted;
//! - `GET /api/user/export/originals` answers the athlete's latest bundle, or
//!   `null`, so a page reopened finds it again;
//! - `GET /api/user/export/originals/{id}` answers the view of the athlete's
//!   own bundle, 404 for any other;
//! - `GET /api/user/export/originals/{id}/download` answers
//!   `{url, expires_at}`, a short signed URL on GCS, or streams the file with
//!   `no-store` from a store that signs nothing; 409 until it is ready.
//!
//! As for the export itself, only the athlete's own first-party session is
//! let in: an API key, an MCP or integration token, a messaging link and an
//! operator's impersonation are refused. A signed URL is a bearer link to the
//! file, so it lives [`DOWNLOAD_TTL`] at most (never past the bundle's own
//! expiry, when an expired bundle is refused) and is handed only to that
//! session.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    Json,
};
use futures_util::TryStreamExt;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::originals_bundle::{
    Admission, BundleDownload, BundleRequest, OriginalsBundleService, DOWNLOAD_TTL,
};
use pierre_services::originals_seal::{self, ExportSeal};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;
use tokio::task;
use tracing::{field::Empty, warn};
use uuid::Uuid;

use crate::data_export::{meter, own_session, refusal, ExportMeter, OwnSession};
use crate::AuthRoutesContext;
use pierre_core::models::TenantId;

/// How many bundles one account may ask for in [`BUNDLE_WINDOW`].
const MAX_BUNDLES_PER_WINDOW: u64 = 3;

/// The window [`MAX_BUNDLES_PER_WINDOW`] is counted in.
const BUNDLE_WINDOW: Duration = Duration::from_hours(24);

/// The cache namespace of the per-account window.
const BUNDLE_RATE_NAMESPACE: &str = "_user_export_bundle_rate";

/// Seconds a busy refusal asks the client to wait.
const BUSY_RETRY_SECS: u64 = 30;

/// What a request for a bundle may carry.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BundleRequestBody {
    /// Seals the bundle with this passphrase; never stored.
    #[serde(default)]
    passphrase: Option<SecretString>,
}

/// The athlete and the tenant their bundles are kept under: a bundle needs
/// one, which an account in no tenant does not have.
async fn bundle_session(
    resources: &AuthRoutesContext,
    headers: &HeaderMap,
) -> AppResult<(Uuid, TenantId)> {
    let OwnSession { user_id, tenant_id } = own_session(resources, headers).await?;
    let tenant_id =
        tenant_id.ok_or_else(|| AppError::invalid_input("No tenant for this session"))?;
    Ok((user_id, tenant_id))
}

/// The service, or the refusal a deployment without a blob store answers.
fn service(resources: &AuthRoutesContext) -> AppResult<&Arc<OriginalsBundleService>> {
    resources.originals_bundles.as_ref().ok_or_else(|| {
        AppError::resource_unavailable(
            "Downloading your original files is not available on this server",
        )
    })
}

/// `POST /api/user/export/originals` — ask for the athlete's originals bundle.
///
/// # Errors
///
/// An authentication or permission error for anything but the athlete's own
/// session; invalid input for a passphrase too short; resource unavailable
/// without a blob store; a database error.
#[tracing::instrument(
    skip(resources, headers, body),
    fields(route = "user_export_originals", user_id = Empty)
)]
pub(crate) async fn handle_request_bundle(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    body: Option<Json<BundleRequestBody>>,
) -> AppResult<Response> {
    let (user_id, tenant_id) = bundle_session(&resources, &headers).await?;
    let bundles = Arc::clone(service(&resources)?);
    let passphrase = body.and_then(|Json(body)| body.passphrase);
    if let Some(passphrase) = &passphrase {
        originals_seal::check_passphrase(passphrase)?;
    }
    // A request a build already in flight, a fresh bundle or a full instance
    // answers costs the athlete nothing; only one that starts a build is
    // counted, and only then is a key derived, inside the build's slot.
    let admission = bundles
        .admit(user_id, tenant_id, passphrase.is_some())
        .await?;
    let slot = match admission {
        Admission::Answered(answer) => return Ok(answered(answer)),
        Admission::Admitted(slot) => slot,
    };
    let used_up = meter(
        &resources,
        user_id,
        ExportMeter {
            namespace: BUNDLE_RATE_NAMESPACE,
            max: MAX_BUNDLES_PER_WINDOW,
            window: BUNDLE_WINDOW,
        },
    )
    .await?;
    if let Some(retry_after) = used_up {
        return Ok(refusal(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_bundles",
            "You have asked for your original files several times today; retry later",
            retry_after,
        ));
    }
    // The key derivation is slow by design: it runs off the async runtime,
    // and the passphrase goes no further than it.
    let seal = match passphrase {
        Some(passphrase) => Some(
            task::spawn_blocking(move || ExportSeal::try_new(&passphrase))
                .await
                .map_err(|e| AppError::internal(format!("sealing a bundle: {e}")))??,
        ),
        None => None,
    };
    Ok(answered(
        bundles.start(slot, user_id, tenant_id, seal).await?,
    ))
}

/// The response a request for a bundle gets.
fn answered(answer: BundleRequest) -> Response {
    match answer {
        BundleRequest::Bundle(view) => (StatusCode::ACCEPTED, Json(view)).into_response(),
        BundleRequest::Busy => refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "export_busy",
            "Bundles are busy; retry in a moment",
            BUSY_RETRY_SECS,
        ),
        BundleRequest::OtherBuilding(view) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "bundle_building",
                "message": "Another bundle is being built; retry once it is ready",
                "bundle": view,
            })),
        )
            .into_response(),
    }
}

/// `GET /api/user/export/originals` — the athlete's latest bundle, `null` when
/// they never asked for one: how a page left and reopened finds it again.
///
/// # Errors
///
/// As [`handle_request_bundle`].
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_export_originals_latest", user_id = Empty)
)]
pub(crate) async fn handle_latest_bundle(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = bundle_session(&resources, &headers).await?;
    let latest = service(&resources)?.latest(user_id, tenant_id).await?;
    Ok(Json(json!(latest)).into_response())
}

/// `GET /api/user/export/originals/{id}` — the athlete's bundle, as they see it.
///
/// # Errors
///
/// As [`handle_request_bundle`]; not found for a bundle that is not theirs.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_export_originals_status", user_id = Empty)
)]
pub(crate) async fn handle_bundle_status(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = bundle_session(&resources, &headers).await?;
    let view = service(&resources)?.view(user_id, tenant_id, id).await?;
    Ok(Json(view).into_response())
}

/// `GET /api/user/export/originals/{id}/download` — the athlete's ready bundle.
///
/// # Errors
///
/// As [`handle_bundle_status`]; a conflict until the bundle is ready; resource
/// unavailable when the store cannot hand it out.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_export_originals_download", user_id = Empty)
)]
pub(crate) async fn handle_bundle_download(
    State(resources): State<AuthRoutesContext>,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
) -> AppResult<Response> {
    let (user_id, tenant_id) = bundle_session(&resources, &headers).await?;
    let download = service(&resources)?
        .download(user_id, tenant_id, id)
        .await?;
    let mut response = match download {
        BundleDownload::Signed { url, expires_at } => {
            // The link is cut short near the bundle's own expiry, so its
            // lifetime is read off the link, not the default.
            let ttl_seconds = (expires_at - chrono::Utc::now())
                .num_seconds()
                .clamp(0, DOWNLOAD_TTL.num_seconds());
            Json(json!({ "url": url, "expires_at": expires_at, "ttl_seconds": ttl_seconds }))
                .into_response()
        }
        BundleDownload::Stream {
            size,
            sealed,
            filename,
            body,
        } => {
            let body = body.map_err(move |e| {
                warn!(bundle_id = %id, error = %e, "Originals bundle stream failed");
                io::Error::other("the bundle stopped")
            });
            let mut response = Response::new(Body::from_stream(body));
            let headers = response.headers_mut();
            let content_type = if sealed {
                "application/octet-stream"
            } else {
                "application/zip"
            };
            headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
            headers.insert(CONTENT_LENGTH, HeaderValue::from(size));
            if let Ok(value) =
                HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
            {
                headers.insert(CONTENT_DISPOSITION, value);
            }
            response
        }
    };
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
