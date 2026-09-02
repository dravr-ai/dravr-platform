// ABOUTME: Axum handler for the user-facing persona cards endpoint
// ABOUTME: Thin shim that delegates to pierre_services::personas pure helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! « Style de coaching » persona cards handler.
//!
//! Thin axum wrapper around [`pierre_services::personas`]. The wire
//! shapes and rendering logic live in `pierre-services`; this module
//! exists only because the axum + `ServerContext` glue is server-local.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;

use pierre_core::errors::AppError;
use pierre_middleware::extract_auth_from_headers;
use pierre_services::personas::{build_personas_response, resolve_persona_locale};

use crate::mcp::resources::ServerContext;

/// Query parameters for [`get_personas_handler`].
#[derive(Debug, Deserialize)]
pub struct ListPersonasQuery {
    /// Optional BCP-47 short locale for the card copy. Unsupported or
    /// absent values fall back to the authenticated user's stored
    /// locale, then to English.
    pub locale: Option<String>,
}

/// Axum handler for `GET /api/personas`.
///
/// Serves one card per [`pierre_core::models::CoachingPersona`] variant,
/// rendered from the live persona-contract registry snapshot so the
/// settings copy always matches the contract the conformance stage
/// enforces. Before the first contremaitre sync the cards degrade to no
/// rules rather than failing.
///
/// # Errors
///
/// - Authentication failures from the middleware extractor.
pub async fn get_personas_handler(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Query(params): Query<ListPersonasQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;

    // The stored profile locale is the fallback when the query parameter
    // is absent or unsupported; a missing user row simply skips to the
    // terminal English fallback.
    let stored_locale = resources
        .common
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .ok()
        .flatten()
        .map(|u| u.locale);
    let locale = resolve_persona_locale(params.locale.as_deref(), stored_locale.as_deref());

    let snapshot = resources.fitness.persona_contract_registry.snapshot();
    let response = build_personas_response(
        &snapshot,
        &resources.mcp.messaging_strings_registry,
        &locale,
    );

    Ok((StatusCode::OK, Json(response)).into_response())
}
