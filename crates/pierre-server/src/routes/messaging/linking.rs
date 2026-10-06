// ABOUTME: Channel linking API handlers for OAuth/deep-link account verification
// ABOUTME: Maps authenticated Pierre users to messaging channel identities (Telegram, Slack, etc.)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, Utc};
use pierre_core::errors::messaging::MessagingError;
use pierre_core::models::messaging::{ChannelType, LinkingMethod, LINK_CODE_TTL_MINUTES};
use pierre_core::models::TenantId;
use pierre_database::backends::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, TenantRepository,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Arc;
use tracing::{error, info, warn};
use uuid::Uuid;

mod destination;
mod link_account;

use super::templates;
use crate::mcp::resources::ServerContext;
use destination::build_linking_url;
pub use destination::can_complete_a_link;
use link_account::resolve_user_from_form;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::AppError;
use pierre_middleware::{extract_auth_from_headers, PeerAddress};
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};

/// Length of the cryptographically random linking code
const LINK_CODE_LENGTH: usize = 32;

/// Characters used for generating linking codes (URL-safe alphanumeric)
const CODE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Query parameters for channel linking callback
#[derive(Debug, Deserialize)]
pub struct LinkCallbackQuery {
    /// State parameter containing the linking verification code
    pub state: Option<String>,
    /// Channel-specific user ID, when the caller already knows it.
    ///
    /// Set by deep-link completions and internal callers. OAuth providers do
    /// not send this — they send `code`, which the exchange below turns into an
    /// identity.
    pub channel_user_id: Option<String>,
    /// Authorization code from an OAuth provider, exchanged for the user's id.
    pub code: Option<String>,
    /// Display name from the platform
    pub display_name: Option<String>,
}

/// Response body for a channel link initiation
#[derive(Debug, Serialize)]
pub struct LinkInitResponse {
    /// Channel type
    pub channel: String,
    /// Linking method used
    pub method: String,
    /// Verification code (for deep link channels)
    pub code: Option<String>,
    /// Linking URL the user should visit/send
    pub linking_url: String,
    /// Expiration timestamp
    pub expires_at: String,
    /// Inline SVG QR code encoding `linking_url` (deep-link channels only, for the
    /// desktop→phone handoff). `None` for OAuth channels or on render failure.
    pub qr_svg: Option<String>,
}

/// Response body for a linked channel
#[derive(Debug, Serialize)]
pub struct ChannelLinkResponse {
    /// Channel type
    pub channel: String,
    /// Channel-specific user identifier
    pub channel_user_id: String,
    /// Display name from the platform
    pub display_name: Option<String>,
    /// When the link was established
    pub linked_at: String,
}

/// Resolve tenant via the canonical resolver. Verifies membership when
/// `active_tenant_id` is claimed; errors if the user has no tenants.
/// No user-id fallback.
async fn resolve_tenant_id(
    auth: &AuthResult,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    require(resolve_tenant(resources, auth, TenantMode::Required).await?)
}

/// Generate a cryptographically random linking code
pub fn generate_link_code() -> String {
    let mut rng = rand::rng();
    (0..LINK_CODE_LENGTH)
        .map(|_| {
            let idx = rng.random_range(0..CODE_CHARSET.len());
            CODE_CHARSET[idx] as char
        })
        .collect()
}

/// Render a QR code for a deep-link URL as an inline SVG string, or `None` on
/// failure. Only deep-link channels (Telegram / `WhatsApp`) get a QR — it bridges the
/// desktop→phone gap when the user onboards on a laptop but runs the chat app only
/// on their phone. Failure is non-fatal: the tappable `linking_url` still works.
fn qr_svg_for(url: &str) -> Option<String> {
    use qrcode::render::svg;
    use qrcode::QrCode;
    let code = QrCode::new(url.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(220, 220)
            .quiet_zone(true)
            .build(),
    )
}

/// POST /api/messaging/link/init/:channel
///
/// Initiates channel linking by generating a verification code and returning
/// a platform-specific linking URL. Requires JWT authentication.
///
/// The bot is the one that serves the caller's tenant, per
/// `MessagingRepository::resolve_channel_config` — the tenant's own when it
/// configured one, otherwise the deployment's platform-scope bot. The link
/// state is written under the tenant that OWNS that config, not the caller's:
/// the webhook resolves its tenant from the same config row and consumes the
/// code with `consume_link_state(code, bot_tenant)`, so a state stored anywhere
/// else is a code the bot can never redeem.
pub async fn init_channel_link(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let caller_tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let method = channel_type.linking_method();
    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(LINK_CODE_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // The bot this link goes through. A disabled config is refused as absent:
    // the webhook only routes to active configs, so its bot could never
    // redeem the code.
    let config = db
        .resolve_channel_config(caller_tenant_id, &channel)
        .await?
        .filter(|config| {
            config
                .get("is_active")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        })
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "No {channel_type} bot is available for your account, so there is \
                 nothing to link to. Choose a channel from the available list."
            ))
        })?;
    let bot_tenant_id = config
        .get("tenant_id")
        .and_then(Value::as_str)
        .and_then(|raw| TenantId::parse_str(raw).ok())
        .ok_or_else(|| AppError::internal("Channel config carries no valid tenant_id"))?;

    // Build the URL before storing the code, so a config that cannot produce
    // one leaves no redeemable state behind.
    let linking_url = build_linking_url(
        channel_type,
        &code,
        &config,
        &resources.common.config.base_url,
    )
    .await?;

    let params = CreateLinkStateParams {
        id: &id,
        tenant_id: bot_tenant_id,
        user_id: Some(&user_id),
        channel_type: &channel,
        code: &code,
        method: &method.to_string(),
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at.to_rfc3339(),
    };
    db.create_link_state(&params).await?;
    let expires_at_str = expires_at.to_rfc3339();

    info!(
        channel = %channel,
        method = %method,
        user_id = %user_id,
        "Initiated channel linking"
    );

    // QR only for deep-link channels — OAuth flows redirect in the same browser.
    let qr_svg = match method {
        LinkingMethod::DeepLink => qr_svg_for(&linking_url),
        LinkingMethod::OAuth => None,
    };

    let response = LinkInitResponse {
        channel: channel_type.to_string(),
        method: method.to_string(),
        code: match method {
            LinkingMethod::DeepLink => Some(code),
            LinkingMethod::OAuth => None,
        },
        linking_url,
        expires_at: expires_at_str,
        qr_svg,
    };

    Ok((StatusCode::OK, Json(json!(response))))
}

/// GET /api/messaging/link/callback/:channel
///
/// Handles OAuth callback or deep-link verification completion.
/// Consumes the verification code and creates a permanent channel link.
pub async fn link_callback(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    Query(query): Query<LinkCallbackQuery>,
) -> Result<impl IntoResponse, AppError> {
    // Validate channel type early (reject unknown channels before DB operations)
    ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let state_code = query
        .state
        .as_deref()
        .ok_or_else(|| AppError::invalid_input("Missing state parameter with linking code"))?;

    // Resolve who this is. A caller that already knows says so; an OAuth
    // provider sends a `code` we exchange. Requiring `channel_user_id`
    // unconditionally is what made every Slack/Discord attempt a 400: the
    // provider never sends it, and nothing else was going to.
    let exchanged;
    let channel_user_id = if query.channel_user_id.is_some() {
        query.channel_user_id.as_deref()
    } else if let Some(oauth_code) = query.code.as_deref() {
        exchanged = exchange_oauth_identity(&resources, &channel, state_code, oauth_code).await?;
        Some(exchanged.as_str())
    } else {
        None
    };
    let channel_user_id = channel_user_id.ok_or_else(|| {
        AppError::invalid_input(
            "Callback carried neither an OAuth code nor a channel user id, so there is \
             no identity to link.",
        )
    })?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // Non-consuming lookup to extract tenant_id for the tenant-scoped consumption
    let preview = db
        .get_link_state(state_code)
        .await?
        .ok_or_else(|| AppError::invalid_input("Link code is invalid or expired"))?;

    let tenant_id_str = preview["tenant_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Link state missing tenant_id"))?;
    let tenant_id = TenantId::parse_str(tenant_id_str)
        .map_err(|_| AppError::internal("Link state has invalid tenant_id"))?;

    // Verify the URL channel matches the link state channel to prevent cross-channel replay
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

    // Atomically consume the link state with tenant_id guard
    let link_state = db.consume_link_state(state_code, tenant_id).await?;

    let user_id = link_state["user_id"]
        .as_str()
        .ok_or_else(|| AppError::invalid_input("Link state has no associated user"))?;

    let link_id = Uuid::new_v4().to_string();
    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id,
        channel_type: &channel,
        channel_user_id,
        display_name: query.display_name.as_deref(),
    };
    db.create_channel_link(&link_params).await?;

    info!(
        channel = %channel,
        user_id = %user_id,
        channel_user_id = %channel_user_id,
        "Channel linked successfully"
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "linked",
            "channel": channel,
            "channel_user_id": channel_user_id,
        })),
    ))
}

/// GET /api/messaging/links
///
/// Lists all linked channels for the authenticated user.
///
/// Authorized by `user_id = auth.user_id` across tenants, not by the caller's
/// tenant: a link made through the deployment bot is stored under the bot's
/// tenant (ingress looks the sender up with `get_channel_link(bot_tenant, ..)`),
/// while the athlete lives in their own. Each row names the person it belongs
/// to and the id comes from the verified session, so only the caller's own
/// links are ever returned.
pub async fn list_channel_links(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let links = db.list_channel_links_for_user(&user_id).await?;

    let response: Vec<ChannelLinkResponse> = links
        .iter()
        .map(|link| ChannelLinkResponse {
            channel: link["channel_type"].as_str().unwrap_or_default().to_owned(),
            channel_user_id: link["channel_user_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            display_name: link["display_name"].as_str().map(String::from),
            linked_at: link["linked_at"].as_str().unwrap_or_default().to_owned(),
        })
        .collect();

    Ok((
        StatusCode::OK,
        Json(json!({
            "tenant_id": tenant_id,
            "links": response
        })),
    ))
}

/// DELETE /api/messaging/links/:channel
///
/// Unlinks a channel for the authenticated user.
///
/// Authorized the way [`list_channel_links`] is: by `user_id = auth.user_id`,
/// across the tenants that hold the caller's links. Every link of this channel
/// that names the caller is removed, each under the tenant that stores it —
/// the athlete asked to be unlinked from the channel, not from one bot's copy.
/// Another user's rows are never reachable: the lookup is keyed on the
/// session's own user id.
pub async fn delete_channel_link(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let mut deleted = false;
    for link in db.list_channel_links_for_user(&user_id).await? {
        if link.get("channel_type").and_then(Value::as_str) != Some(channel.as_str()) {
            continue;
        }
        let tenant_id = link
            .get("tenant_id")
            .and_then(Value::as_str)
            .and_then(|raw| TenantId::parse_str(raw).ok())
            .ok_or_else(|| AppError::internal("Channel link carries no valid tenant_id"))?;
        deleted |= db
            .delete_channel_link(tenant_id, &user_id, &channel)
            .await?;
    }

    if !deleted {
        return Err(MessagingError::ChannelNotLinked {
            channel: channel_type.to_string(),
        }
        .into());
    }

    info!(
        channel = %channel,
        user_id = %user_id,
        "Channel unlinked"
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "unlinked",
            "channel": channel_type.to_string()
        })),
    ))
}

// ════════════════════════════════════════════════════════════════
// Webhook-Initiated Channel Linking (HTML pages, no auth required)
// ════════════════════════════════════════════════════════════════

/// Form data submitted from the channel link login/register page
#[derive(Debug, Deserialize)]
pub struct ChannelLinkAuthForm {
    /// The link code from the hidden form field
    pub code: String,
    /// Email address
    pub email: String,
    /// Password
    pub password: String,
    /// "login" or "register"
    pub action: String,
    /// Display name (only for registration)
    pub display_name: Option<String>,
}

/// GET /messaging/link/:code
///
/// Renders the login/register page for a webhook-initiated channel link.
/// Public endpoint, no authentication required.
pub async fn channel_link_page(
    State(resources): State<Arc<ServerContext>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    let Some(link_state) = db.get_link_state(&code).await.ok().flatten() else {
        return templates::render_link_error_page(
            "This link has expired or is invalid. Please send a new message to the bot to get a fresh link.",
        ).into_response();
    };

    let channel = link_state["channel_type"].as_str().unwrap_or("messaging");
    let sender_name = link_state["sender_name"].as_str();

    templates::render_link_login_page(channel, sender_name, &code, None).into_response()
}

/// POST /messaging/link/auth
///
/// Handles login or registration from the channel link page.
/// On success, completes the link and renders the success page.
/// On failure, re-renders the login page with an error message.
pub async fn channel_link_auth(
    State(resources): State<Arc<ServerContext>>,
    peer: PeerAddress,
    headers: HeaderMap,
    Form(form): Form<ChannelLinkAuthForm>,
) -> impl IntoResponse {
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // Validate the link code is still valid
    let Some(link_state) = db.get_link_state(&form.code).await.ok().flatten() else {
        return templates::render_link_error_page(
            "This link has expired or is invalid. Please send a new message to the bot to get a fresh link.",
        ).into_response();
    };

    let channel = link_state["channel_type"]
        .as_str()
        .unwrap_or("messaging")
        .to_owned();
    let sender_name = link_state["sender_name"].as_str();
    let channel_user_id = link_state["channel_user_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let tenant_id_str = link_state["tenant_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let Ok(tenant_id) = TenantId::parse_str(&tenant_id_str) else {
        return templates::render_link_error_page("Internal error: invalid tenant").into_response();
    };

    // Authenticate or register
    let limiter = &resources.auth.oauth2_rate_limiter;
    let client = peer.0.map(|peer| limiter.client_address(peer, &headers));
    let resolved = resolve_user_from_form(&resources, &form, client).await;
    let (user_id, user_status) = match resolved {
        Ok(resolved) => resolved,
        Err(msg) => {
            return templates::render_link_login_page(
                &channel,
                sender_name,
                &form.code,
                Some(&msg),
            )
            .into_response();
        }
    };

    // For login (existing user), verify the bot that issued this code serves
    // them (see `bot_serves_user`). For register (new user), skip — they were
    // just created and have no tenants yet.
    if form.action != "register" {
        let serves = match bot_serves_user(&resources, user_id, tenant_id, &channel).await {
            Ok(serves) => serves,
            Err(e) => {
                error!(error = %e, "Failed to check the bot's reach during link auth");
                return templates::render_link_error_page("An error occurred. Please try again.")
                    .into_response();
            }
        };

        if !serves {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                "Channel's bot does not serve this user"
            );
            return templates::render_link_login_page(
                &channel,
                sender_name,
                &form.code,
                Some("Cannot link to this channel. Your account does not belong to this organization."),
            )
            .into_response();
        }
    }

    // Complete the link state and create permanent channel link
    complete_link_and_respond(
        db,
        &form.code,
        user_id,
        tenant_id,
        &channel,
        &channel_user_id,
        sender_name,
    )
    .await
    .map_or_else(
        |refusal| templates::render_link_error_page(refusal).into_response(),
        |()| templates::render_link_success_page(&channel, user_status).into_response(),
    )
}

/// Whether the bot that issued a link code — the config `channel` holds under
/// `bot_tenant_id`, the tenant the link state and the link itself are stored
/// under — may link `user_id`.
///
/// A bot serves the members of the tenant that owns it, and every user whose
/// own tenant it serves per `MessagingRepository::resolve_channel_config`: the
/// tenant's own config for the channel when it has one, otherwise the
/// deployment's platform-scope bot. That is the rule Settings offers channels
/// and issues codes by, so an athlete in a personal tenant links through the
/// deployment bot here exactly as through the in-chat OTP flow — the link is
/// stored under the bot's tenant and names the athlete. A bot a tenant keeps
/// to itself serves no one outside that tenant.
async fn bot_serves_user(
    resources: &ServerContext,
    user_id: Uuid,
    bot_tenant_id: TenantId,
    channel: &str,
) -> Result<bool, AppError> {
    let tenants: &dyn TenantRepository = resources.common.repos.tenants.as_ref();
    if tenants
        .get_user_role(user_id, bot_tenant_id)
        .await?
        .is_some()
    {
        return Ok(true);
    }
    let messaging: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    for tenant in tenants.list_for_user(user_id).await? {
        let serving = messaging.resolve_channel_config(tenant.id, channel).await?;
        let serving_tenant = serving
            .as_ref()
            .and_then(|config| config.get("tenant_id"))
            .and_then(Value::as_str)
            .and_then(|raw| TenantId::parse_str(raw).ok());
        if serving_tenant == Some(bot_tenant_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Complete the link state and create the permanent channel link, or return
/// the words the error page says why it could not be made in
async fn complete_link_and_respond(
    db: &dyn MessagingRepository,
    code: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    channel: &str,
    channel_user_id: &str,
    sender_name: Option<&str>,
) -> Result<(), &'static str> {
    let user_id_str = user_id.to_string();

    // Complete the link state (set user_id, mark used)
    if let Err(e) = db.complete_link_state(code, &user_id_str).await {
        warn!(error = %e, code = %code, "Failed to complete link state");
        return Err("This link has already been used or has expired. Please request a new link.");
    }

    // Create the permanent channel link
    let link_id = Uuid::new_v4().to_string();
    let link_params = CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id: &user_id_str,
        channel_type: channel,
        channel_user_id,
        display_name: sender_name,
    };

    if let Err(e) = db.create_channel_link(&link_params).await {
        warn!(error = %e, "Failed to create channel link after auth");
        return Err("This channel identity is already linked to an account.");
    }

    info!(
        channel = %channel,
        user_id = %user_id_str,
        channel_user_id = %channel_user_id,
        "Channel linked via webhook-initiated auth flow"
    );

    Ok(())
}

/// Exchange an OAuth authorization code for the sender's id on that platform.
///
/// This is the half that was missing. The callback used to demand a
/// `channel_user_id` query parameter that no OAuth provider sends, so Slack and
/// Discord links always failed — the provider is the only party that knows who
/// just authorised, and nothing was asking it.
///
/// Identity only: the token is used once to read the id and never stored. We are
/// not acting on the user's behalf on Slack or Discord, so keeping a credential
/// that would let us would be holding risk with no purpose.
///
/// # Errors
///
/// Returns `AppError` when the channel is not an OAuth channel, its credentials
/// are missing, or the provider rejects the exchange. The message never carries
/// the provider's raw body — a rejected exchange can echo the client secret back.
async fn exchange_oauth_identity(
    resources: &Arc<ServerContext>,
    channel: &str,
    link_code: &str,
    oauth_code: &str,
) -> Result<String, AppError> {
    let channel_type = ChannelType::from_str(channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    // Two different codes arrive on this callback and they are not
    // interchangeable: `link_code` is our own state token, which is what the
    // link-state row is keyed by, and `oauth_code` is the provider's
    // authorization code, which only the provider can resolve. Looking the
    // tenant up by the provider's code never matches — it is not a key we
    // issued — so that mistake fails every OAuth link with "invalid or
    // expired" no matter how the channel is configured.
    let tenant_str = db
        .get_link_state(link_code)
        .await
        .ok()
        .flatten()
        .and_then(|s| s["tenant_id"].as_str().map(str::to_owned))
        .unwrap_or_default();
    let tenant_id = TenantId::parse_str(&tenant_str)
        .map_err(|_| AppError::invalid_input("Link code is invalid or expired"))?;

    let config = db
        .get_channel_config(tenant_id, channel)
        .await?
        .ok_or_else(|| AppError::internal("Channel is not configured"))?;

    let client_id = config
        .get("api_key")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::internal("Channel has no OAuth client id"))?;
    let client_secret = config
        .get("api_secret")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| AppError::internal("Channel has no OAuth client secret"))?;

    let base_url = &resources.common.config.base_url;
    let redirect_uri = format!("{base_url}/api/messaging/link/callback/{channel_type}");

    let (token_url, identity_url) = oauth_endpoints(channel_type)?;

    exchange_code_for_identity(
        token_url,
        identity_url,
        client_id,
        client_secret,
        oauth_code,
        &redirect_uri,
        channel,
    )
    .await
}

/// The token and identity endpoints for an OAuth channel.
///
/// Split out so the pairing is assertable on its own, and so the round trip
/// below can be pointed at a stub without reaching for a config knob that would
/// exist only for tests.
///
/// # Errors
///
/// Returns `AppError` for a channel that does not link by OAuth.
pub fn oauth_endpoints(
    channel_type: ChannelType,
) -> Result<(&'static str, &'static str), AppError> {
    match channel_type {
        ChannelType::Slack => Ok((
            "https://slack.com/api/openid.connect.token",
            "https://slack.com/api/openid.connect.userInfo",
        )),
        ChannelType::Discord => Ok((
            "https://discord.com/api/oauth2/token",
            "https://discord.com/api/users/@me",
        )),
        other => Err(AppError::invalid_input(format!(
            "{other} does not link by OAuth"
        ))),
    }
}

/// Exchange an authorization code for the sender's id against the given
/// endpoints.
///
/// Takes the endpoints as parameters rather than deriving them, which is what
/// makes the round trip testable: the production caller passes the real Slack or
/// Discord `URLs`, and a test passes a local stub speaking the same shapes. That
/// covers the parts most likely to be wrong — form encoding, bearer auth, the
/// `sub`-or-`id` extraction, and every failure branch — without needing real
/// OAuth apps.
///
/// # Errors
///
/// Returns `AppError` when the request fails, the response is not JSON, the
/// provider returns no `access_token`, or the identity carries no user id. The
/// message never includes the provider's body — a rejected exchange echoes back
/// parameters that include the client secret.
pub async fn exchange_code_for_identity(
    token_url: &str,
    identity_url: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    channel: &str,
) -> Result<String, AppError> {
    let client = reqwest::Client::new();
    let token_response = client
        .post(token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| {
            warn!(error = %e, channel = %channel, "OAuth token exchange request failed");
            AppError::internal("Could not complete the OAuth exchange")
        })?;

    let token_json: Value = token_response.json().await.map_err(|e| {
        warn!(error = %e, channel = %channel, "OAuth token response was not JSON");
        AppError::internal("Could not complete the OAuth exchange")
    })?;

    let access_token = token_json
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            // Deliberately not logging the body: a rejected exchange echoes back
            // parameters that can include the client secret.
            warn!(channel = %channel, "OAuth token response carried no access_token");
            AppError::internal("The OAuth provider rejected the exchange")
        })?;

    let identity: Value = client
        .get(identity_url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| {
            warn!(error = %e, channel = %channel, "identity lookup failed");
            AppError::internal("Could not read the account identity")
        })?
        .json()
        .await
        .map_err(|e| {
            warn!(error = %e, channel = %channel, "identity response was not JSON");
            AppError::internal("Could not read the account identity")
        })?;

    // Slack OIDC returns the stable user id as `sub`; Discord returns `id`.
    identity
        .get("sub")
        .or_else(|| identity.get("id"))
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            warn!(channel = %channel, "identity response carried no user id");
            AppError::internal("Could not read the account identity")
        })
}
