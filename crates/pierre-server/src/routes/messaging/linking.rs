// ABOUTME: Channel linking API handlers for OAuth/deep-link account verification
// ABOUTME: Maps authenticated Pierre users to messaging channel identities (Telegram, Slack, etc.)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{Duration, Utc};
use pierre_core::errors::messaging::MessagingError;
use pierre_core::models::messaging::{ChannelType, LinkingMethod, LINK_CODE_TTL_MINUTES};
use pierre_core::models::{TenantId, User};
use pierre_database::backends::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, TenantRepository,
    UserRepository,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::{LazyLock, RwLock};
use tokio::task::spawn_blocking;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::templates;
use crate::mcp::resources::ServerContext;
use pierre_auth::auth::AuthResult;
use pierre_config::utils::http_client::shared_client;
use pierre_core::errors::AppError;
use pierre_middleware::extract_auth_from_headers;
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

/// Cache of bot token -> username, so `getMe` is called once per token rather
/// than once per link request. A bot's username changes only when an operator
/// renames it in `BotFather`, and a stale entry would send codes to a handle that
/// no longer resolves, so the process lifetime is the right bound: a redeploy
/// re-reads it.
static TELEGRAM_BOT_USERNAMES: LazyLock<RwLock<HashMap<String, String>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The Telegram bot a link code must be sent to.
///
/// Asked of Telegram, not configured. `getMe` returns the username belonging to
/// the bot token we already store, which makes the answer correct by
/// construction — there is no value for an operator to set, mistype, or leave
/// stale, and no second place for it to drift from.
///
/// That matters more here than it usually would. This URL is where the athlete
/// sends a one-time code that binds whoever sends it to their account, so a
/// wrong handle is not a broken link, it is a credential disclosure. The
/// previous implementation defaulted to a hardcoded `PierreBot` — a real bot
/// belonging to a stranger — and every link went there, because the config
/// write path never persisted a `bot_username` for the default to fall back
/// from. An environment variable would have fixed that instance while leaving
/// the same shape in place: a human-supplied name that nothing verifies.
///
/// A missing or rejected token is an error. Guessing is never correct.
async fn telegram_bot_username(config: &serde_json::Value) -> Result<String, AppError> {
    let token = config
        .get("bot_token")
        .and_then(|v| v.as_str())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::internal(
                "Telegram channel has no bot_token, so the bot's username cannot be \
                 resolved and no linking URL can be built. Configure the channel \
                 before issuing link codes.",
            )
        })?;

    if let Some(cached) = TELEGRAM_BOT_USERNAMES
        .read()
        .ok()
        .and_then(|m| m.get(token).cloned())
    {
        return Ok(cached);
    }

    let url = format!("https://api.telegram.org/bot{token}/getMe");
    let response = shared_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::internal(format!("Telegram getMe request failed: {e}")))?;

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| AppError::internal(format!("Telegram getMe returned no JSON: {e}")))?;

    let username = body
        .get("result")
        .and_then(|r| r.get("username"))
        .and_then(|u| u.as_str())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| {
            // Deliberately does not log the body: a rejected token comes back
            // with a description that can echo the token itself.
            AppError::internal(
                "Telegram getMe did not return a username — the stored bot_token is \
                 rejected or the bot was deleted. Refusing to build a linking URL \
                 without knowing which bot it points at.",
            )
        })?
        .to_owned();

    if let Ok(mut cache) = TELEGRAM_BOT_USERNAMES.write() {
        cache.insert(token.to_owned(), username.clone());
    }
    info!(bot_username = %username, "resolved the Telegram bot username via getMe");
    Ok(username)
}

/// Build the linking URL based on channel type and method.
async fn build_linking_url(
    channel_type: ChannelType,
    code: &str,
    config: &serde_json::Value,
    base_url: &str,
) -> Result<String, AppError> {
    match channel_type {
        ChannelType::Telegram => {
            // No fallback bot, deliberately. This used to default to
            // "PierreBot" when the channel config carried no `bot_username` —
            // and it always did, because the config write path never persists
            // that key (see `config.rs`, which extracts api_key / api_secret /
            // webhook_secret / verify_token / account_id / phone_number /
            // bot_token and nothing else). So EVERY link pointed at
            // https://t.me/PierreBot, which is a real bot belonging to a
            // stranger.
            //
            // That is worse than a dead link. `detect_linking_code` +
            // `execute_link_code` bind whoever sends the code to the requesting
            // athlete's account inside the TTL, so pressing Start on that
            // third-party bot hands an account-binding credential off-platform.
            //
            // Guessing a bot name is therefore never acceptable: an absent
            // username must fail loudly rather than produce a plausible URL
            // aimed at someone else's bot.
            let bot_username = telegram_bot_username(config).await?;
            Ok(format!("https://t.me/{bot_username}?start={code}"))
        }
        ChannelType::WhatsApp => {
            // Same rule as Telegram above, for the same reason. An empty number
            // yields `https://wa.me/?text=LINK+CODE`, which is not a dead link:
            // WhatsApp opens the contact picker with the athlete's one-time
            // binding code already typed, and whoever they pick receives it.
            // Milder than aiming at a stranger's bot only because it takes a
            // tap — the failure is identical in kind, so it fails identically.
            let phone = config
                .get("phone_number")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    AppError::internal(
                        "WhatsApp channel has no phone_number, so there is no recipient \
                         for the link code. Refusing to build a URL that would open a \
                         contact picker with the athlete's binding code pre-filled.",
                    )
                })?;
            let message_text = format!("LINK {code}");
            let encoded_message = urlencoding::encode(&message_text);
            Ok(format!("https://wa.me/{phone}?text={encoded_message}"))
        }
        ChannelType::Messenger => {
            // Same rule as Telegram and WhatsApp above: no guessed target. The
            // page id identifies which Messenger page the link opens, and an
            // absent one previously produced `.../link/callback/messenger?state=`
            // — our own endpoint, which rejects the request for the
            // `channel_user_id` nothing supplies. That is the 400 every Messenger
            // link attempt hit.
            //
            // `ref` is Messenger's own deep-link parameter and comes back on the
            // webhook (dravr-canot >= 0.4.20 parses it from both the bare
            // `referral` and the `postback.referral` shape), which is what makes
            // it the equivalent of Telegram's `?start=`.
            let page_id = config
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    AppError::internal(
                        "Messenger channel has no account_id, so there is no page for the \
                         link to open. Refusing to build a URL that cannot complete.",
                    )
                })?;
            Ok(format!("https://m.me/{page_id}?ref={code}"))
        }
        // Genuine OAuth channels. These used to return our OWN callback with only
        // a `state` param — an endpoint that rejects the request for the
        // `channel_user_id` nothing supplies, so every attempt 400'd. The user
        // has to be sent to the provider first; the provider is what knows who
        // they are, which is the whole point of the round trip.
        //
        // `api_key` carries the OAuth client id. The picker already refuses to
        // advertise a channel whose credentials are absent, and this refuses to
        // build a URL for one anyway — the same refuse-to-guess rule the
        // deep-link channels follow.
        ChannelType::Slack | ChannelType::Discord => {
            let client_id = config
                .get("api_key")
                .and_then(|v| v.as_str())
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    AppError::internal(format!(
                        "{channel_type} channel has no OAuth client id, so the authorize URL \
                         cannot identify this app. Refusing to build a link that cannot complete."
                    ))
                })?;

            let redirect_uri = format!("{base_url}/api/messaging/link/callback/{channel_type}");
            // Identity only. Linking needs to learn who the person is and
            // nothing else; a broader scope would ask for consent we have no
            // use for, which is both a worse prompt and more to leak.
            let (authorize, scope) = match channel_type {
                ChannelType::Slack => ("https://slack.com/openid/connect/authorize", "openid"),
                _ => ("https://discord.com/oauth2/authorize", "identify"),
            };

            Ok(format!(
                "{authorize}?response_type=code&client_id={}&scope={}&redirect_uri={}&state={}",
                urlencoding::encode(client_id),
                urlencoding::encode(scope),
                urlencoding::encode(&redirect_uri),
                urlencoding::encode(code),
            ))
        }
    }
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
pub async fn init_channel_link(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let method = channel_type.linking_method();
    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(LINK_CODE_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();

    // Fetch channel config for building the linking URL
    let config = db
        .get_channel_config(tenant_id, &channel)
        .await?
        .unwrap_or(json!({}));

    let params = CreateLinkStateParams {
        id: &id,
        tenant_id,
        user_id: Some(&user_id),
        channel_type: &channel,
        code: &code,
        method: &method.to_string(),
        channel_user_id: None,
        sender_name: None,
        expires_at: &expires_at.to_rfc3339(),
    };
    db.create_link_state(&params).await?;

    let linking_url = build_linking_url(
        channel_type,
        &code,
        &config,
        &resources.common.config.base_url,
    )
    .await?;
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
pub async fn list_channel_links(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let links = db.list_user_channel_links(tenant_id, &user_id).await?;

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
pub async fn delete_channel_link(
    State(resources): State<Arc<ServerContext>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let deleted = db
        .delete_channel_link(tenant_id, &user_id, &channel)
        .await?;

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
    let user_id = resolve_user_from_form(&resources, &form).await;
    let user_id = match user_id {
        Ok(uid) => uid,
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

    // For login (existing user), verify they belong to the link state's tenant.
    // For register (new user), skip — they were just created and have no tenants yet.
    if form.action != "register" {
        let tenant_repo: &dyn TenantRepository = resources.common.repos.tenants.as_ref();
        let has_role = tenant_repo
            .get_user_role(user_id, tenant_id)
            .await
            .ok()
            .flatten();

        if has_role.is_none() {
            warn!(
                user_id = %user_id,
                tenant_id = %tenant_id,
                "User does not belong to the channel's tenant"
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
}

/// Resolve user identity from form data (login or register)
async fn resolve_user_from_form(
    resources: &ServerContext,
    form: &ChannelLinkAuthForm,
) -> Result<Uuid, String> {
    match form.action.as_str() {
        "register" => register_user(resources, form).await,
        _ => authenticate_user(resources, &form.email, &form.password).await,
    }
}

/// Complete the link state and create the permanent channel link, returning an HTML response
async fn complete_link_and_respond(
    db: &dyn MessagingRepository,
    code: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    channel: &str,
    channel_user_id: &str,
    sender_name: Option<&str>,
) -> Response {
    let user_id_str = user_id.to_string();

    // Complete the link state (set user_id, mark used)
    if let Err(e) = db.complete_link_state(code, &user_id_str).await {
        warn!(error = %e, code = %code, "Failed to complete link state");
        return templates::render_link_error_page(
            "This link has already been used or has expired. Please request a new link.",
        )
        .into_response();
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
        return templates::render_link_error_page(
            "This channel identity is already linked to an account.",
        )
        .into_response();
    }

    info!(
        channel = %channel,
        user_id = %user_id_str,
        channel_user_id = %channel_user_id,
        "Channel linked via webhook-initiated auth flow"
    );

    templates::render_link_success_page(channel).into_response()
}

/// Authenticate a user by email and password
///
/// Returns the user ID on success, or an error message string on failure.
async fn authenticate_user(
    resources: &ServerContext,
    email: &str,
    password: &str,
) -> Result<Uuid, String> {
    let user_repo: &dyn UserRepository = resources.common.repos.users.as_ref();

    let user = user_repo
        .get_by_email(email)
        .await
        .map_err(|e| {
            error!(error = %e, "Database error during link auth");
            "An error occurred. Please try again.".to_owned()
        })?
        .ok_or_else(|| "Invalid email or password.".to_owned())?;

    // Verify password using bcrypt with spawn_blocking
    let password_owned = password.to_owned();
    let hash_owned = user.password_hash.clone();

    let is_valid = spawn_blocking(move || bcrypt::verify(&password_owned, &hash_owned))
        .await
        .map_err(|_| "An error occurred. Please try again.".to_owned())?
        .map_err(|_| "An error occurred. Please try again.".to_owned())?;

    if !is_valid {
        return Err("Invalid email or password.".to_owned());
    }

    Ok(user.id)
}

/// Register a new user and return their ID
///
/// Creates the user account. Returns a user-facing error on failure.
async fn register_user(
    resources: &ServerContext,
    form: &ChannelLinkAuthForm,
) -> Result<Uuid, String> {
    let user_repo: &dyn UserRepository = resources.common.repos.users.as_ref();

    // Check for existing user
    let existing = user_repo.get_by_email(&form.email).await.map_err(|e| {
        error!(error = %e, "Database error during registration");
        "An error occurred. Please try again.".to_owned()
    })?;

    if existing.is_some() {
        return Err("An account with this email already exists. Please log in instead.".to_owned());
    }

    // Hash password using bcrypt with spawn_blocking
    let password_owned = form.password.clone();
    let password_hash = spawn_blocking(move || bcrypt::hash(&password_owned, bcrypt::DEFAULT_COST))
        .await
        .map_err(|_| "An error occurred. Please try again.".to_owned())?
        .map_err(|_| "An error occurred. Please try again.".to_owned())?;

    let display_name = form
        .display_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(String::from);

    let user = User::new(form.email.clone(), password_hash, display_name);
    let user_id = user.id;

    user_repo.create(&user).await.map_err(|e| {
        error!(error = %e, "Failed to create user during link registration");
        "An error occurred. Please try again.".to_owned()
    })?;

    info!(user_id = %user_id, email = %form.email, "User registered via channel link auth");

    Ok(user_id)
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
