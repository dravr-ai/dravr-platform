// ABOUTME: POST /oauth2/token-validate — the SDK's check of the token and client it holds, answered by introspection
// ABOUTME: A token is valid exactly when RFC 7662 introspection finds it active; the response keeps the SDK's {valid, error} shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/oauth2/token-validate` lets a client (the SDK,
//! `sdk/src/oauth-session-manager.ts`) check the access token and client it
//! holds before it reconnects. It is a thin caller of the one introspection
//! implementation ([`OAuth2Routes::introspect`]): a token is valid exactly
//! when introspection finds it active, so a token whose consent grant was
//! withdrawn or expired is invalid here too. The route and its response
//! shape stay for the SDK until it introspects itself.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_database::backends::OAuth2ServerRepository;
use serde_json::{json, Value};
use tracing::{debug, error, info};

use super::{OAuth2Context, OAuth2Routes};

/// What the token in a request's `Authorization` header is.
enum Presented {
    /// No `Authorization` header
    None,
    /// The header is not a Bearer token
    Malformed(&'static str),
    /// A Bearer token
    Bearer(String),
}

/// The `Authorization` header of `headers`, read as a Bearer token.
fn presented_token(headers: &HeaderMap) -> Presented {
    let Some(header) = headers.get(header::AUTHORIZATION) else {
        return Presented::None;
    };
    let Ok(header) = header.to_str() else {
        return Presented::Malformed("Invalid Authorization header encoding");
    };
    header.strip_prefix("Bearer ").map_or(
        Presented::Malformed("Authorization header must use Bearer scheme"),
        |token| Presented::Bearer(token.to_owned()),
    )
}

/// A 200 answer carrying `body`, which the SDK reads.
fn answer(body: &Value) -> Response {
    (StatusCode::OK, Json(body)).into_response()
}

impl OAuth2Routes {
    /// Handle token validation request (POST /oauth2/token-validate)
    pub(super) async fn handle_token_validate(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Json(request): Json<Value>,
    ) -> Response {
        debug!("Token validation request received");

        // Extract client_id from request body (optional)
        let client_id = request.get("client_id").and_then(|v| v.as_str());

        let token_valid = match Self::presented_token_valid(&context, &headers).await {
            Ok(valid) => valid,
            Err(answered) => return *answered,
        };

        // Validate client_id if provided
        if let Some(cid) = client_id {
            return Self::validate_client_id_response(context.oauth2_server, cid).await;
        }

        if token_valid {
            answer(&json!({ "valid": true }))
        } else {
            answer(&json!({
                "valid": false,
                "error": "invalid_request",
                "error_description": "Either access token or client_id must be provided"
            }))
        }
    }

    /// Whether the request's Bearer token is active, as introspection finds
    /// it: `false` when it presents none, and the answer itself for a
    /// malformed header, an inactive token or a failed check.
    async fn presented_token_valid(
        context: &OAuth2Context,
        headers: &HeaderMap,
    ) -> Result<bool, Box<Response>> {
        let token = match presented_token(headers) {
            // No token provided - not an error, just not validated
            Presented::None => return Ok(false),
            Presented::Malformed(description) => {
                return Err(Box::new(answer(&json!({
                    "valid": false,
                    "error": "invalid_request",
                    "error_description": description
                }))));
            }
            Presented::Bearer(token) => token,
        };
        match Self::introspect(context, &token).await {
            Ok(introspection) if introspection.active => Ok(true),
            Ok(_) => {
                debug!("Token validation: the token is not active");
                Err(Box::new(answer(&json!({
                    "valid": false,
                    "error": "invalid_token",
                    "error_description": "Access token is invalid or expired"
                }))))
            }
            // Nothing was decided about the token: no `valid` member, so the
            // SDK keeps its credentials and retries.
            Err(e) => {
                error!("Token validation could not introspect the token: {e}");
                Err(Box::new(
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        Json(json!({
                            "error": "temporarily_unavailable",
                            "error_description": "The token could not be checked right now"
                        })),
                    )
                        .into_response(),
                ))
            }
        }
    }

    /// Validate `client_id` and return appropriate response
    async fn validate_client_id_response(
        oauth2_server: Arc<dyn OAuth2ServerRepository>,
        client_id: &str,
    ) -> Response {
        let client_manager = ClientRegistrationManager::new(oauth2_server);
        match client_manager.get_client(client_id).await {
            Ok(_) => {
                info!(
                    "Credentials validated successfully for client_id: {}",
                    client_id
                );
                answer(&json!({ "valid": true }))
            }
            Err(e) => {
                ClientRegistrationManager::log_lookup_failure(client_id, &e);
                answer(&json!({
                    "valid": false,
                    "error": "invalid_client",
                    "error_description": "Client ID not found or invalid"
                }))
            }
        }
    }
}
