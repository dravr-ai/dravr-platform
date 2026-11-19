// ABOUTME: What a completed account sign-in does to its pair's OAuth grant: replaces a dead one
// ABOUTME: A sign-in ends at `connected`: it never sends the athlete on to the provider's own authorization
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A sign-in and its pair's grant (carnet#1494).
//!
//! A sign-in over a dead grant deletes the grant's rows and keeps its data
//! ([`replace_dead_grant`], JF's directive 3).
//!
//! A completed sign-in answers `connected` and nothing more: the connect ends
//! there, and no client is sent on to the provider's own authorization.
//! Garmin's grant, which only workout push and calendar writes need, is asked
//! for when such a write is refused for it
//! (`backend_resolver::grant_owed_refusal`), with its authorization link.

use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver;
use tracing::{info, warn};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The `connected` answer to a sign-in of `backend`. `short_circuit` when a
/// session stored moments ago answered it without a new login.
pub(crate) fn connected_response(backend: &str, short_circuit: bool) -> Response {
    let mut answer = serde_json::json!({
        "status": "connected",
        "provider": backend,
    });
    if short_circuit {
        answer["short_circuit"] = serde_json::Value::Bool(true);
    }
    Json(answer).into_response()
}

/// Delete the dead OAuth grant a sign-in of `backend` just replaced, keeping
/// everything it fetched (carnet#1494, JF's directive 3).
///
/// A grant whose refresh the provider refused reconnects only through this
/// hosted sign-in, and the session now serves the provider, so the grant's
/// token row and its connection row go. Nothing is revoked upstream and no
/// data is purged: the activities and wellness it stored stay the athlete's.
/// A live grant beside the session is left alone, as the session's fallback.
/// A delete that fails is logged; the session serves either way, and the dead
/// grant is never elected while its connection needs a reconnect.
pub(crate) async fn replace_dead_grant(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
) {
    let Some(grant) =
        backend_resolver::flagged_grant(&resources.repos, user_id, tenant_id, backend).await
    else {
        return;
    };
    let deleted = match resources
        .repos
        .oauth_tokens
        .delete_token(user_id, tenant_id, grant)
        .await
    {
        Ok(()) => {
            resources
                .repos
                .provider_connections
                .remove_connection(user_id, tenant_id, grant)
                .await
        }
        Err(e) => Err(e),
    };
    match deleted {
        Ok(()) => info!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            grant,
            backend,
            "Sign-in over a dead grant: the grant's token and connection are deleted, its data kept"
        ),
        Err(e) => warn!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            grant,
            backend,
            error = %e,
            "Sign-in over a dead grant: the grant's rows could not be deleted; the session serves"
        ),
    }
}
