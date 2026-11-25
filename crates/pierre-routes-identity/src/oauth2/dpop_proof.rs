// ABOUTME: The DPoP proof (RFC 9449) a token request or a pushed authorization request carries, verified once
// ABOUTME: Answers the proven key's thumbprint, or the 400 invalid_dpop_proof a bad, repeated or replayed proof earns

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::HeaderMap;
use pierre_auth::dpop::{DpopRefusal, PresentedProofs, DPOP_HEADER};
use pierre_auth::oauth2_server::models::OAuth2Error;
use tracing::warn;

use super::{OAuth2Context, OAuth2Routes};

impl OAuth2Routes {
    /// The key a request's `DPoP` proof was signed with, verified for `POST`
    /// to `path` on the issuer (RFC 9449 §5, §10.1) and its `jti` burned;
    /// `None` when the request carries no `DPoP` header.
    ///
    /// # Errors
    /// `invalid_dpop_proof` for more than one proof, or one that fails a
    /// check or was already used; `server_error` when the replay memory
    /// cannot be reached, which refuses rather than admits.
    pub(super) async fn proven_dpop_key(
        context: &OAuth2Context,
        headers: &HeaderMap,
        path: &str,
    ) -> Result<Option<String>, OAuth2Error> {
        let values = headers.get_all(DPOP_HEADER);
        let count = values.iter().count();
        if count == 0 {
            return Ok(None);
        }
        let proofs = PresentedProofs {
            proof: values.iter().next().and_then(|value| value.to_str().ok()),
            count,
        };
        let htu = format!("{}{path}", context.config.issuer_url.trim_end_matches('/'));
        context
            .dpop_proofs
            .verify(proofs, "POST", Some(&htu), None)
            .await
            .map(|proof| Some(proof.jkt))
            .map_err(|refusal| {
                warn!(path, refusal = %refusal, "DPoP proof refused");
                refused_proof(&refusal)
            })
    }
}

/// The OAuth error a refused proof is answered with.
fn refused_proof(refusal: &DpopRefusal) -> OAuth2Error {
    if refusal.is_server_fault() {
        return OAuth2Error::server_error("DPoP proofs cannot be checked right now");
    }
    OAuth2Error::invalid_dpop_proof(&refusal.to_string())
}
