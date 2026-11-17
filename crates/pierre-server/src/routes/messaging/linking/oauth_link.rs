// ABOUTME: Completes a Slack/Discord OAuth link only for the Dravr account that started it
// ABOUTME: A signed-in browser vouches on the web; the phone app takes the code back and finishes with its own login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Binding an OAuth link to the account that started it (carnet#1580).
//!
//! The link code in `state` names the Dravr account; the provider's `code`
//! names the channel identity. Without a third check, anyone holding someone
//! else's authorize URL completes it: the account that started the link sends
//! the URL to another person, that person approves on Slack or Discord, and
//! their channel identity is linked to the sender, whose account then answers
//! their DMs. So a link completes only for a caller signed in as the account
//! that started it:
//!
//! - **Web:** the provider redirects the browser to the callback, which carries
//!   the web app's session cookie (the callback is on the web app's origin).
//!   The callback completes the link when that session is the starter's.
//! - **Phone:** the app opens the provider in an auth session that has no Dravr
//!   session, so the callback hands the provider's code back to the app at the
//!   return URI recorded when the app started the link — always the app's own
//!   fixed callback, checked at init — and the app finishes the link through
//!   [`complete_app_link`] with its bearer token.
//!
//! Every refusal happens before the provider is called and leaves the link
//! state unconsumed.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Extension, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_database::backends::{CreateChannelLinkParams, MessagingRepository};
use pierre_middleware::extract_auth_from_headers;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use serde::Deserialize;
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::exchange_oauth_identity;
use crate::mcp::resources::ServerContext;

/// Body of `POST /api/messaging/link/complete/{channel}`: what the phone app
/// took back from the callback.
#[derive(Deserialize)]
pub(crate) struct LinkCompleteBody {
    /// The link code the app's link started with, echoed by the provider.
    pub state: String,
    /// The provider's authorization code.
    pub code: String,
}

impl fmt::Debug for LinkCompleteBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LinkCompleteBody")
            .field("state", &"[REDACTED]")
            .field("code", &"[REDACTED]")
            .finish()
    }
}

/// A live OAuth link state, read without consuming it.
pub(super) struct PendingOAuthLink {
    /// The tenant the state, and the link, are stored under.
    pub tenant_id: TenantId,
    /// The account that started the link.
    pub starter: Uuid,
    /// Where the phone app takes the provider's code back, when the app
    /// started the link.
    pub app_return_uri: Option<String>,
}

/// Read the live link state `state_code` names, for `channel`.
///
/// # Errors
///
/// Refuses an unknown, used or expired code, a code started for another
/// channel, and a state with no starting account.
pub(super) async fn pending_oauth_link(
    db: &dyn MessagingRepository,
    state_code: &str,
    channel: &str,
) -> Result<PendingOAuthLink, AppError> {
    let preview = db
        .get_link_state(state_code)
        .await?
        .ok_or_else(|| AppError::invalid_input("Link code is invalid or expired"))?;

    let tenant_id = preview["tenant_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Link state missing tenant_id"))
        .and_then(|raw| {
            TenantId::parse_str(raw)
                .map_err(|_| AppError::internal("Link state has invalid tenant_id"))
        })?;

    // The URL channel must match the state's, so a code cannot be replayed
    // through another channel's callback.
    let stored_channel = preview["channel_type"].as_str().unwrap_or_default();
    if stored_channel != channel {
        warn!(
            url_channel = %channel,
            stored_channel = %stored_channel,
            "Channel mismatch between URL and link state"
        );
        return Err(AppError::invalid_input(format!(
            "Channel mismatch: link was created for {stored_channel}, not {channel}"
        )));
    }

    // Only an OAuth link is finished by a provider's code.
    if preview["method"].as_str() != Some("oauth") {
        return Err(AppError::invalid_input(
            "Link code was not started as an OAuth link",
        ));
    }

    let starter = preview["user_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .ok_or_else(|| AppError::invalid_input("Link state has no associated user"))?;

    Ok(PendingOAuthLink {
        tenant_id,
        starter,
        app_return_uri: preview["app_return_uri"].as_str().map(str::to_owned),
    })
}

/// The account the request's session is signed in as, `None` when it carries
/// no session or one that no longer authenticates.
///
/// # Errors
///
/// Passes a spent request budget and a server fault through: neither says
/// that nobody is signed in.
pub(super) async fn signed_in_account(
    resources: &Arc<ServerContext>,
    headers: &HeaderMap,
) -> Result<Option<Uuid>, AppError> {
    match extract_auth_from_headers(headers, resources).await {
        Ok(auth) => Ok(Some(auth.user_id)),
        Err(e) if e.code == ErrorCode::RateLimitExceeded || e.is_server_fault() => Err(e),
        Err(_) => Ok(None),
    }
}

/// Refuse a caller who is not signed in as the account that started the link.
///
/// # Errors
///
/// Returns `PermissionDenied` when `caller` is `None` or another account.
pub(super) fn require_starter(
    caller: Option<Uuid>,
    pending: &PendingOAuthLink,
) -> Result<(), AppError> {
    if caller == Some(pending.starter) {
        return Ok(());
    }
    warn!(
        starter = %pending.starter,
        signed_in = caller.is_some(),
        "Refused an OAuth link completed outside the session that started it"
    );
    // The refusal names no account: the caller may be the person the link
    // was forwarded to.
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "This link was started from another Dravr account, or from a browser that is not signed in to it. Start the link again from the Dravr app you want to link.",
    ))
}

/// Send the provider's code back to the phone app that started the link.
///
/// Nothing is exchanged or consumed here: the app finishes the link with its
/// own login. `app_return_uri` was checked against the app's fixed callback
/// when the link started, which carries no query, so the code reaches only
/// Dravr's app. A declined authorization goes back too, as
/// `error=access_denied`, so the app's sign-in sheet closes instead of
/// stranding the athlete on a refusal page; the provider's own error text is
/// never reflected.
pub(super) fn hand_back_to_app(
    app_return_uri: &str,
    channel: &str,
    state_code: &str,
    answer: ProviderAnswer<'_>,
) -> Response {
    let outcome = match answer {
        ProviderAnswer::Approved(oauth_code) => format!("code={}", urlencoding::encode(oauth_code)),
        ProviderAnswer::Declined => "error=access_denied".to_owned(),
    };
    let target = format!(
        "{app_return_uri}?channel={}&state={}&{outcome}",
        urlencoding::encode(channel),
        urlencoding::encode(state_code),
    );
    (StatusCode::FOUND, [(header::LOCATION, target)]).into_response()
}

/// What the provider's redirect says about the authorize step.
#[derive(Clone, Copy)]
pub(super) enum ProviderAnswer<'a> {
    /// The athlete approved: the provider's authorization code.
    Approved(&'a str),
    /// The provider sent no code: the athlete declined, or the provider refused.
    Declined,
}

impl fmt::Debug for ProviderAnswer<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Approved(_) => f.write_str("Approved([REDACTED])"),
            Self::Declined => f.write_str("Declined"),
        }
    }
}

/// Exchange the provider's code, consume the link state and store the link.
///
/// The caller has already established that the account completing the link is
/// the one that started it.
///
/// # Errors
///
/// Returns `AppError` when the provider refuses the code, the state was
/// consumed meanwhile, or the link cannot be stored.
pub(super) async fn complete_oauth_link(
    resources: &Arc<ServerContext>,
    adapters: &dyn ChannelAdapterFactory,
    channel: &str,
    state_code: &str,
    oauth_code: &str,
    pending: &PendingOAuthLink,
) -> Result<Value, AppError> {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let identity =
        exchange_oauth_identity(resources, adapters, channel, pending.tenant_id, oauth_code)
            .await?;
    let channel_user_id = identity.id.as_str();

    // Atomically consume the link state with tenant_id guard
    db.consume_link_state(state_code, pending.tenant_id).await?;

    let user_id = pending.starter.to_string();
    let link_id = Uuid::new_v4().to_string();
    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id: pending.tenant_id,
        user_id: &user_id,
        channel_type: channel,
        channel_user_id,
        display_name: identity.display_name.as_deref(),
    };
    db.create_channel_link(&link_params).await?;

    info!(
        channel = %channel,
        user_id = %user_id,
        channel_user_id = %channel_user_id,
        "Channel linked successfully"
    );

    Ok(json!({
        "status": "linked",
        "channel": channel,
        "channel_user_id": channel_user_id,
    }))
}

/// POST /api/messaging/link/complete/:channel
///
/// Finishes an OAuth link the phone app started: the app took the provider's
/// code back from the callback and presents it with its own login, which must
/// be the account that started the link.
///
/// # Errors
///
/// Refuses a missing or lapsed login (401, so the app renews its session and
/// retries), an unknown channel, an unknown or expired link code, and a caller
/// who did not start the link, before the provider is called.
pub(crate) async fn complete_app_link(
    State(resources): State<Arc<ServerContext>>,
    Extension(adapters): Extension<Arc<dyn ChannelAdapterFactory>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(body): Json<LinkCompleteBody>,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let pending = pending_oauth_link(db, &body.state, &channel).await?;
    require_starter(Some(auth.user_id), &pending)?;

    let linked = complete_oauth_link(
        &resources,
        adapters.as_ref(),
        &channel,
        &body.state,
        &body.code,
        &pending,
    )
    .await?;
    Ok((StatusCode::OK, Json(linked)))
}
