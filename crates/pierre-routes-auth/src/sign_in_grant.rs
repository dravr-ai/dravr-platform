// ABOUTME: What a completed account sign-in does to its pair's OAuth grant: replaces a dead one, then asks for the grant it still owes
// ABOUTME: Garmin connects and reconnects by its hosted sign-in first, then its OAuth grant, which pushes workouts and writes the calendar
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A sign-in and its pair's grant (carnet#1494).
//!
//! A sign-in over a dead grant deletes the grant's rows and keeps its data
//! ([`replace_dead_grant`], JF's directive 3). Then the follow-on
//! authorization (JF's directive 8):
//!
//! Garmin's account sign-in (`sciotte_garmin`) reads everything the athlete
//! records, and only Garmin's own OAuth grant pushes workouts and writes the
//! calendar (`backend_resolver::grant_writes_beside_mirror`). So a Garmin
//! connect or reconnect signs in first and authorizes the grant right after.
//! The one decision lives here, on the answer every sign-in client reads
//! (`POST /api/providers/sciotte/{login,select-2fa,submit-otp}`): a
//! `connected` answer carries `oauth_follow_on` while the grant is owed
//! (`backend_resolver::grant_owed`) and Garmin's client has a seat for the
//! athlete. The web and mobile sign-in and the hosted connect picker start
//! that authorization their own way; the hosted sign-in a provider's own
//! link opens holds no token that can start it, so its answer carries the
//! authorization URL, minted here, returning to that page's success screen.
//!
//! Strava's grant writes nothing its session cannot, so a Strava sign-in is
//! never followed by its OAuth.

use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::models::TenantId;
use pierre_middleware::provider_link_token::CONNECT_PROVIDER;
use pierre_providers::backend_resolver;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::card_seats::require_garmin_seat;
use crate::oauth::require_oauth_start_notice;
use crate::sciotte::LinkContext;
use crate::AuthRoutesContext;

/// The authorization a completed sign-in is followed by.
#[derive(Debug, Serialize)]
pub(crate) struct OAuthFollowOn {
    /// The provider whose OAuth to start (`garmin`).
    pub(crate) provider: &'static str,
    /// The authorization URL itself, for the hosted sign-in a provider's own
    /// link opened, whose token cannot start an authorization. Absent for
    /// every other client, which starts it its own way.
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_url: Option<String>,
}

/// The `connected` answer to a sign-in of `backend`, with the authorization
/// it is followed by ([`oauth_follow_on`]). `short_circuit` when a session
/// stored moments ago answered it without a new login: a sign-in repeated
/// after its grant's authorization was left unfinished still goes on to it.
pub(crate) async fn connected_response(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    backend: &str,
    link: Option<&LinkContext>,
    short_circuit: bool,
) -> Response {
    let follow_on = oauth_follow_on(
        resources,
        user_id,
        TenantId::from_uuid(tenant_id),
        backend,
        link,
    )
    .await;
    if let Some(follow_on) = &follow_on {
        info!(
            user_id = %user_id,
            backend,
            grant = follow_on.provider,
            "Sign-in completed; its grant's authorization follows"
        );
    }
    let mut answer = serde_json::json!({
        "status": "connected",
        "provider": backend,
        "oauth_follow_on": follow_on,
    });
    if short_circuit {
        answer["short_circuit"] = serde_json::Value::Bool(true);
    }
    Json(answer).into_response()
}

/// The authorization a completed sign-in of `backend` is followed by: the
/// pair's grant while it is owed for its writes and a seat is there for the
/// athlete, else `None`. A grant with no seat left stays owed; the write it
/// would carry is refused with its authorization link later. The hosted
/// picker reads it too, for a connected card that still owes its grant.
pub(crate) async fn oauth_follow_on(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    backend: &str,
    link: Option<&LinkContext>,
) -> Option<OAuthFollowOn> {
    let grant = backend_resolver::grant_owed(
        &resources.repos,
        &resources.provider_registry,
        user_id,
        tenant_id,
        backend,
    )
    .await?;
    require_garmin_seat(resources, user_id, tenant_id, grant)
        .await
        .ok()?;
    let authorization_url = match link {
        Some(link) if link.target != CONNECT_PROVIDER => {
            mint_follow_on_url(resources, user_id, tenant_id, grant, link).await
        }
        _ => None,
    };
    Some(OAuthFollowOn {
        provider: grant,
        authorization_url,
    })
}

/// The authorization URL of `grant`, returning to the hosted sign-in's success
/// page for `link`'s channel, or `None` when its notice is outstanding or the
/// URL cannot be minted.
async fn mint_follow_on_url(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: TenantId,
    grant: &str,
    link: &LinkContext,
) -> Option<String> {
    require_oauth_start_notice(resources, user_id, tenant_id, grant, false)
        .await
        .ok()?;
    let success = format!(
        "{}/providers/signin/success?channel={}&target={grant}",
        resources.config.base_url.trim_end_matches('/'),
        urlencoding::encode(&link.channel),
    );
    OAuthService::new(resources.data.clone(), resources.config.clone())
        .get_auth_url(
            user_id,
            tenant_id,
            grant,
            AuthUrlOptions {
                return_redirect: Some(&success),
            },
        )
        .await
        .inspect_err(|e| {
            warn!(
                user_id = %user_id,
                grant,
                error = %e,
                "Sign-in completed; its grant's authorization URL could not be minted"
            );
        })
        .ok()
        .map(|authorization| authorization.authorization_url)
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
