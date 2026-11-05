// ABOUTME: The JWKS endpoint — the RS256 public keys a client verifies tokens with
// ABOUTME: Serves the key set with an ETag, so a client holding the current version gets a 304
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::State,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_auth::admin::jwks::JsonWebKeySet;
use sha2::{Digest, Sha256};
use tokio::task::spawn_blocking;
use tracing::{debug, error};

use super::{OAuth2Context, OAuth2Routes};

impl OAuth2Routes {
    /// Handle JWKS endpoint (GET /oauth2/jwks or GET /.well-known/jwks.json)
    pub(super) async fn handle_jwks(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
    ) -> Response {
        // Return JWKS with RS256 public keys for token validation
        let jwks = match context.jwks_manager.get_jwks() {
            Ok(jwks) => jwks,
            Err(e) => {
                error!("Failed to generate JWKS: {}", e);
                // Return empty JWKS on error (graceful degradation)
                return (
                    StatusCode::OK,
                    [(header::CACHE_CONTROL, "public, max-age=3600")],
                    Json(serde_json::json!({ "keys": [] })),
                )
                    .into_response();
            }
        };

        debug!("JWKS endpoint accessed, returning {} keys", jwks.keys.len());

        // Calculate ETag from JWKS content for efficient caching
        let (_jwks_json, etag) = match Self::compute_jwks_etag(jwks.clone()).await {
            Ok(result) => result,
            Err(response) => return *response,
        };

        // Check if client's cached version matches current version
        if Self::check_etag_match(&headers, &etag) {
            debug!("JWKS ETag match, returning 304 Not Modified");
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
        }

        // Return JWKS with ETag and Cache-Control headers
        (
            StatusCode::OK,
            [
                (header::CACHE_CONTROL, "public, max-age=3600".to_owned()),
                (header::ETAG, etag),
            ],
            Json(jwks),
        )
            .into_response()
    }

    // ============================================================================
    // Helper Functions
    // ============================================================================

    /// Compute JWKS `ETag` from JSON content
    async fn compute_jwks_etag(jwks: JsonWebKeySet) -> Result<(String, String), Box<Response>> {
        let etag_result = spawn_blocking(move || {
            let jwks_json = serde_json::to_string(&jwks)?;
            let mut hasher = Sha256::new();
            hasher.update(jwks_json.as_bytes());
            let hash = hasher.finalize();
            let etag = format!(r#""{}""#, hex::encode(&hash[..16]));
            Ok::<(String, String), serde_json::Error>((jwks_json, etag))
        })
        .await;

        match etag_result {
            Ok(Ok((json, tag))) => Ok((json, tag)),
            Ok(Err(_)) => {
                error!("Failed to serialize JWKS for ETag calculation");
                Err(Box::new(Self::jwks_error_response()))
            }
            Err(_) => {
                error!("Spawn blocking task panicked during JWKS serialization");
                Err(Box::new(Self::jwks_error_response()))
            }
        }
    }

    /// Create error response for JWKS endpoint
    fn jwks_error_response() -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "keys": []
            })),
        )
            .into_response()
    }

    /// Check if client has current JWKS version (`ETag` match)
    fn check_etag_match(headers: &HeaderMap, etag: &str) -> bool {
        headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|client_etag| client_etag == etag)
    }
}
