// ABOUTME: Super-admin review of the fuel-product bank: the proposal queue, its label images, approve (with edits) and reject, and the live catalogue
// ABOUTME: The catalogue is global reference data, so every route is super-admin only and none is tenant-scoped (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fuel-product review
//!
//! Facts enter the fuel catalogue only through a proposal an operator
//! approves here. The catalogue (`fuel_products`, `fuel_product_versions`,
//! `fuel_product_proposals`) is global reference data — the same for every
//! tenant — so these routes carry no tenant and check none: there is no
//! `require_tenant_access` because there is no tenant to compare against.
//! Every route is super-admin only instead, reads included, since a change
//! here reaches every athlete on the platform.
//!
//! An approval can edit the proposed facts first: each field of
//! [`FuelFactOverrides`] keeps, sets or clears its value, the edited fields
//! are recorded with provenance `admin_edit`, and the merged facts pass the
//! plausibility gate again before anything is written. Each
//! [`FuelApprovalOutcome`] answers with its own status and an `outcome` code
//! the console branches on.
//!
//! The label image is served from the stored bytes rather than linked from
//! the brand's CDN: the console's content-security policy only loads images
//! from its own origin.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{
    CatalogueFuelListing, CatalogueFuelProduct, FuelApprovalOutcome, FuelFactOverrides,
    FuelProductProposal, FuelRejectReason, PlausibilityViolation, ProposalStatus,
};

use crate::context::AdminApiContext;

/// Longest rejection note accepted, in characters.
pub const MAX_REJECT_NOTE_CHARS: usize = 2000;

/// Rows a list returns when the request names no limit.
const DEFAULT_PAGE: i64 = 50;

/// The most rows one list returns.
const MAX_PAGE: i64 = 100;

/// The label image types the console is served. Anything else stored is
/// refused rather than sent under a type a browser might sniff into script.
pub const SERVABLE_IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Refuse anyone but a super-admin: the catalogue is global.
fn require_super_admin(admin_token: &ValidatedAdminToken) -> AppResult<()> {
    if admin_token.is_super_admin {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "The fuel-product catalogue is global: super-admin privileges required",
        ))
    }
}

fn parse_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw)
        .map_err(|_| AppError::invalid_input(format!("Invalid fuel proposal id: {raw}")))
}

fn page(limit: Option<i64>, offset: Option<i64>) -> (i64, i64) {
    (
        limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE),
        offset.unwrap_or(0).max(0),
    )
}

/// A JSON body with a status.
fn reply(status: StatusCode, body: serde_json::Value) -> Response {
    (status, Json(body)).into_response()
}

/// Query of `GET /api/admin/fuel-products/proposals`.
#[derive(Debug, Default, Deserialize)]
pub struct ProposalListQuery {
    /// `pending` (the default), `approved` or `rejected`.
    pub status: Option<String>,
    /// Rows to return, clamped to 1..=100; 50 when absent.
    pub limit: Option<i64>,
    /// Rows to skip.
    pub offset: Option<i64>,
}

/// Body of `GET /api/admin/fuel-products/proposals`.
#[derive(Debug, Serialize)]
pub struct ProposalListResponse {
    /// The proposals, oldest first, without their images.
    pub proposals: Vec<FuelProductProposal>,
    /// The status listed.
    pub status: ProposalStatus,
    /// The limit applied.
    pub limit: i64,
    /// The offset applied.
    pub offset: i64,
}

/// Handle `GET /api/admin/fuel-products/proposals`.
///
/// # Errors
///
/// 403 for anyone but a super-admin, 400 for an unknown status, or a
/// repository error.
pub async fn handle_list_fuel_proposals(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<ProposalListQuery>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let status = match query.status.as_deref().map(str::trim) {
        None | Some("") => ProposalStatus::Pending,
        Some(raw) => ProposalStatus::parse(raw).ok_or_else(|| {
            AppError::invalid_input(format!(
                "status must be one of pending, approved, rejected, got `{raw}`"
            ))
        })?,
    };
    let (limit, offset) = page(query.limit, query.offset);
    let proposals = context
        .repos
        .fuel_catalogue
        .list_fuel_proposals(status, limit, offset)
        .await?;
    Ok(Json(ProposalListResponse {
        proposals,
        status,
        limit,
        offset,
    })
    .into_response())
}

/// Body of `GET /api/admin/fuel-products/proposals/{id}`.
#[derive(Debug, Serialize)]
pub struct ProposalDetailResponse {
    /// The proposal.
    pub proposal: FuelProductProposal,
    /// The stored label image's size in bytes, when one is stored; the image
    /// itself is served by the `source-image` route.
    pub source_image_bytes: Option<usize>,
}

/// Handle `GET /api/admin/fuel-products/proposals/{id}`.
///
/// # Errors
///
/// 403 for anyone but a super-admin, 400 for a malformed id, 404 for an
/// unknown one, or a repository error.
pub async fn handle_get_fuel_proposal(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(proposal_id): Path<String>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let id = parse_id(&proposal_id)?;
    let detail = context
        .repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Fuel proposal {id}")))?;
    Ok(Json(ProposalDetailResponse {
        source_image_bytes: detail.source_image.as_ref().map(|i| i.bytes.len()),
        proposal: detail.proposal,
    })
    .into_response())
}

/// Handle `GET /api/admin/fuel-products/proposals/{id}/source-image`.
///
/// The stored label image, with its stored type when that is one of
/// [`SERVABLE_IMAGE_TYPES`], never cached and never type-sniffed.
///
/// # Errors
///
/// 403 for anyone but a super-admin, 400 for a malformed id, 404 for an
/// unknown proposal or one with no image, 415 for a stored type outside the
/// allowlist, or a repository error.
pub async fn handle_get_fuel_proposal_image(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(proposal_id): Path<String>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let id = parse_id(&proposal_id)?;
    let image = context
        .repos
        .fuel_catalogue
        .get_fuel_proposal(id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Fuel proposal {id}")))?
        .source_image
        .ok_or_else(|| AppError::not_found(format!("Label image of fuel proposal {id}")))?;
    let mime = image.mime.trim().to_ascii_lowercase();
    let Some(content_type) = SERVABLE_IMAGE_TYPES.iter().find(|t| **t == mime) else {
        return Ok(reply(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            json!({
                "outcome": "unsupported_image_type",
                "error": format!("The stored label image is `{}`, which is not served", image.mime),
            }),
        ));
    };
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("default-src 'none'"),
            ),
        ],
        image.bytes,
    )
        .into_response())
}

/// Body of `POST /api/admin/fuel-products/proposals/{id}/approve`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproveFuelProposalRequest {
    /// The operator's edits: a field left out keeps the proposed value, a
    /// value replaces it, `null` clears it. Empty for a plain approval.
    #[serde(default)]
    pub overrides: FuelFactOverrides,
}

/// One gate violation as the console shows it.
#[derive(Debug, Serialize)]
pub struct ViolationView {
    /// The stable code ([`PlausibilityViolation::code`]).
    pub code: &'static str,
    /// What is wrong, in a sentence.
    pub message: String,
    /// The violation's own fields.
    pub detail: PlausibilityViolation,
}

impl From<PlausibilityViolation> for ViolationView {
    fn from(violation: PlausibilityViolation) -> Self {
        Self {
            code: violation.code(),
            message: violation.to_string(),
            detail: violation,
        }
    }
}

/// The response an approval outcome gets: its own status, and an `outcome`
/// code the console branches on.
///
/// | Outcome | Status | `outcome` |
/// |---|---|---|
/// | approved | 200 | `approved`, with `product` |
/// | merged facts fail the gate | 422 | `implausible`, with `violations` |
/// | already reviewed | 410 | `not_pending`, with `status` |
/// | GTIN and named product disagree | 409 | `identity_conflict`, with `product_id` |
/// | no such proposal | 404 | `not_found` |
#[must_use]
pub fn approval_response(outcome: FuelApprovalOutcome) -> Response {
    match outcome {
        FuelApprovalOutcome::Approved(product) => reply(
            StatusCode::OK,
            json!({ "outcome": "approved", "product": *product }),
        ),
        FuelApprovalOutcome::Implausible(violations) => {
            let violations: Vec<ViolationView> =
                violations.into_iter().map(ViolationView::from).collect();
            reply(
                StatusCode::UNPROCESSABLE_ENTITY,
                json!({
                    "outcome": "implausible",
                    "error": "The facts fail the plausibility gate; nothing was written",
                    "violations": violations,
                }),
            )
        }
        FuelApprovalOutcome::NotPending(status) => reply(
            StatusCode::GONE,
            json!({
                "outcome": "not_pending",
                "error": format!("The proposal was already {}", status.as_str()),
                "status": status,
            }),
        ),
        FuelApprovalOutcome::IdentityConflict { product_id } => reply(
            StatusCode::CONFLICT,
            json!({
                "outcome": "identity_conflict",
                "error": "The proposal's barcode belongs to another catalogue product than the one it names",
                "product_id": product_id,
            }),
        ),
        FuelApprovalOutcome::NotFound => reply(
            StatusCode::NOT_FOUND,
            json!({ "outcome": "not_found", "error": "No fuel proposal has that id" }),
        ),
    }
}

/// The signed-in operator a review is attributed to. A service token names
/// no person, and a catalogue version always records who approved it.
fn reviewer(admin_token: &ValidatedAdminToken) -> AppResult<Uuid> {
    admin_token.operator_user_id.ok_or_else(|| {
        AppError::new(
            ErrorCode::PermissionDenied,
            "Reviewing a fuel proposal needs a signed-in operator; a service token names no person",
        )
    })
}

/// Handle `POST /api/admin/fuel-products/proposals/{id}/approve`.
///
/// See [`approval_response`] for what each outcome answers.
///
/// # Errors
///
/// 403 for anyone but a signed-in super-admin, 400 for a malformed id or
/// body, or a repository error.
pub async fn handle_approve_fuel_proposal(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(proposal_id): Path<String>,
    Json(body): Json<ApproveFuelProposalRequest>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let reviewer = reviewer(&admin_token)?;
    let id = parse_id(&proposal_id)?;
    let outcome = context
        .repos
        .fuel_catalogue
        .approve_fuel_proposal(id, reviewer, &body.overrides)
        .await?;
    if let FuelApprovalOutcome::Approved(product) = &outcome {
        info!(
            token_id = %admin_token.token_id,
            proposal_id = %id,
            product_id = %product.product.id,
            version_id = %product.current.id,
            "admin approved fuel proposal"
        );
    }
    Ok(approval_response(outcome))
}

/// Body of `POST /api/admin/fuel-products/proposals/{id}/reject`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectFuelProposalRequest {
    /// `illegible`, `wrong_product`, `implausible`, `duplicate`, `not_fuel`
    /// or `other`.
    pub reason: String,
    /// What the operator saw; required for `other`.
    #[serde(default)]
    pub note: Option<String>,
}

/// Handle `POST /api/admin/fuel-products/proposals/{id}/reject`.
///
/// 200 with the rejected proposal; 404 (`not_found`) for an unknown id and
/// 410 (`not_pending`) for one already reviewed, the way an approval
/// answers them.
///
/// # Errors
///
/// 403 for anyone but a signed-in super-admin, 400 for a malformed id, an
/// unknown reason, a note over [`MAX_REJECT_NOTE_CHARS`] or `other` without
/// one, or a repository error.
pub async fn handle_reject_fuel_proposal(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(proposal_id): Path<String>,
    Json(body): Json<RejectFuelProposalRequest>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let reviewer = reviewer(&admin_token)?;
    let id = parse_id(&proposal_id)?;
    let reason = FuelRejectReason::parse(body.reason.trim()).ok_or_else(|| {
        AppError::invalid_input(format!(
            "reason must be one of illegible, wrong_product, implausible, duplicate, not_fuel, other, got `{}`",
            body.reason
        ))
    })?;
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    if note.is_some_and(|n| n.chars().count() > MAX_REJECT_NOTE_CHARS) {
        return Err(AppError::invalid_input(format!(
            "note must be at most {MAX_REJECT_NOTE_CHARS} characters"
        )));
    }
    if reason == FuelRejectReason::Other && note.is_none() {
        return Err(AppError::invalid_input(
            "A rejection for reason `other` needs a note saying what",
        ));
    }
    let catalogue = &context.repos.fuel_catalogue;
    let rejected = catalogue
        .reject_fuel_proposal(id, reviewer, reason, note)
        .await?;
    let Some(detail) = catalogue.get_fuel_proposal(id).await? else {
        return Ok(approval_response(FuelApprovalOutcome::NotFound));
    };
    if !rejected {
        return Ok(approval_response(FuelApprovalOutcome::NotPending(
            detail.proposal.status,
        )));
    }
    info!(
        token_id = %admin_token.token_id,
        proposal_id = %id,
        reason = reason.as_str(),
        "admin rejected fuel proposal"
    );
    Ok(reply(
        StatusCode::OK,
        json!({ "outcome": "rejected", "proposal": detail.proposal }),
    ))
}

/// Query of `GET /api/admin/fuel-products`.
#[derive(Debug, Default, Deserialize)]
pub struct CatalogueListQuery {
    /// Rows to return, clamped to 1..=100; 50 when absent.
    pub limit: Option<i64>,
    /// Rows to skip.
    pub offset: Option<i64>,
}

/// One live catalogue product as the console lists it.
#[derive(Debug, Serialize)]
pub struct CatalogueRow {
    /// The product with its current version.
    #[serde(flatten)]
    pub product: CatalogueFuelProduct,
    /// How many versions its label has had, the current one included.
    pub version_count: i64,
}

impl From<CatalogueFuelListing> for CatalogueRow {
    fn from(listing: CatalogueFuelListing) -> Self {
        Self {
            product: listing.product,
            version_count: listing.version_count,
        }
    }
}

/// Body of `GET /api/admin/fuel-products`.
#[derive(Debug, Serialize)]
pub struct CatalogueListResponse {
    /// The live products, in brand/name/variant order.
    pub products: Vec<CatalogueRow>,
    /// The limit applied.
    pub limit: i64,
    /// The offset applied.
    pub offset: i64,
}

/// Handle `GET /api/admin/fuel-products`: what athletes can carry today.
///
/// # Errors
///
/// 403 for anyone but a super-admin, or a repository error.
pub async fn handle_list_fuel_catalogue(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<CatalogueListQuery>,
) -> AppResult<Response> {
    require_super_admin(&admin_token)?;
    let (limit, offset) = page(query.limit, query.offset);
    let products = context
        .repos
        .fuel_catalogue
        .list_fuel_catalogue(limit, offset)
        .await?
        .into_iter()
        .map(CatalogueRow::from)
        .collect();
    Ok(Json(CatalogueListResponse {
        products,
        limit,
        offset,
    })
    .into_response())
}
