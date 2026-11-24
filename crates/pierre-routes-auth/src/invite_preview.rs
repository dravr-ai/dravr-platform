// ABOUTME: GET /api/invites/{code} — the public invite card a /groups/join/<code> link shows before any form (carnet#1649)
// ABOUTME: No session needed, metered per address, one 404 for every unknown code, and nothing in the body but who, which group and whether it works

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The invite card.
//!
//! A `/groups/join/<code>` link used to open on a sign-up form that never said
//! who had sent it. This route answers first, for a reader who may have no
//! account: the group's name, who sent the link ("Jeremy B."), the kind of
//! invite and whether it still works. The code is a bearer secret, so the body
//! carries no id and no member count, an unknown code is a 404 identical for
//! every code, and each address is metered like a code guess.
//!
//! The route works signed in or out. A valid session only changes whether the
//! opening is counted: the people running the group do not count themselves
//! (see [`pierre_services::group_invite_card::preview`]). A session that
//! cannot be read counts as no session, never as a refusal.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::Timestamp;

use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::{AppError, AppResult};
use pierre_middleware::PeerAddress;
use pierre_services::group_invite_card;

use crate::pending_session::session_reader;
use crate::signup_code::meter_code_attempt;
use crate::signup_guards::client_address;
use crate::AuthRoutesContext;

/// `GET /api/invites/{code}` — the card for the invite `code` names.
///
/// # Errors
///
/// Returns a rate-limit error past the per-address allowance, not-found for a
/// blank or unknown code (the same body for every one), and the repository's
/// error when a lookup fails.
pub(crate) async fn handle_invite_preview(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> AppResult<Response> {
    let client = client_address(&resources.rate_limiter, peer, &headers);
    meter_code_attempt(
        &resources.rate_limiter,
        OAuth2Endpoint::InvitePreview,
        client,
    )
    .await?;

    let viewer = session_reader(&resources, &headers)
        .await
        .ok()
        .map(|user| user.id);
    let card = group_invite_card::preview(&resources.repos, &code, viewer, Timestamp::now())
        .await?
        .ok_or_else(|| AppError::not_found("Invite"))?;

    // Each read is counted, so no cache may answer in its place.
    let mut response = Json(card).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
