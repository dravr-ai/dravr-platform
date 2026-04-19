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
use pierre_database::plugins::{
    CreateChannelLinkParams, CreateLinkStateParams, MessagingRepository, TenantRepository,
    UserRepository,
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::templates;
use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::middleware::extract_auth_from_headers;
use pierre_auth::auth::AuthResult;

/// Length of the cryptographically random linking code
const LINK_CODE_LENGTH: usize = 32;

/// Characters used for generating linking codes (URL-safe alphanumeric)
const CODE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Query parameters for channel linking callback
#[derive(Debug, Deserialize)]
pub struct LinkCallbackQuery {
    /// State parameter containing the linking verification code
    pub state: Option<String>,
    /// Channel-specific user ID (resolved from OAuth token exchange or deep link)
    pub channel_user_id: Option<String>,
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

/// Resolve tenant ID from auth result
fn resolve_tenant_id(auth: &AuthResult) -> TenantId {
    auth.active_tenant_id
        .map_or_else(|| TenantId::from(auth.user_id), TenantId::from)
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

/// Build the linking URL based on channel type and method
fn build_linking_url(
    channel_type: ChannelType,
    code: &str,
    config: &serde_json::Value,
    base_url: &str,
) -> String {
    match channel_type {
        ChannelType::Telegram => {
            let bot_username = config
                .get("bot_username")
                .and_then(|v| v.as_str())
                .unwrap_or("PierreBot");
            format!("https://t.me/{bot_username}?start={code}")
        }
        ChannelType::WhatsApp => {
            let phone = config
                .get("phone_number")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let message_text = format!("LINK {code}");
            let encoded_message = urlencoding::encode(&message_text);
            format!("https://wa.me/{phone}?text={encoded_message}")
        }
        // OAuth channels return the full callback URL so callers get a usable absolute URL
        ChannelType::Slack | ChannelType::Discord | ChannelType::Messenger => {
            format!("{base_url}/api/messaging/link/callback/{channel_type}?state={code}")
        }
    }
}

/// POST /api/messaging/link/init/:channel
///
/// Initiates channel linking by generating a verification code and returning
/// a platform-specific linking URL. Requires JWT authentication.
pub async fn init_channel_link(
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let method = channel_type.linking_method();
    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(LINK_CODE_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

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

    let linking_url = build_linking_url(channel_type, &code, &config, &resources.config.base_url);
    let expires_at_str = expires_at.to_rfc3339();

    info!(
        channel = %channel,
        method = %method,
        user_id = %user_id,
        "Initiated channel linking"
    );

    let response = LinkInitResponse {
        channel: channel_type.to_string(),
        method: method.to_string(),
        code: match method {
            LinkingMethod::DeepLink => Some(code),
            LinkingMethod::OAuth => None,
        },
        linking_url,
        expires_at: expires_at_str,
    };

    Ok((StatusCode::OK, Json(json!(response))))
}

/// GET /api/messaging/link/callback/:channel
///
/// Handles OAuth callback or deep-link verification completion.
/// Consumes the verification code and creates a permanent channel link.
pub async fn link_callback(
    State(resources): State<Arc<ServerResources>>,
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

    let channel_user_id = query
        .channel_user_id
        .as_deref()
        .ok_or_else(|| AppError::invalid_input("Missing channel_user_id parameter"))?;

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    // Non-consuming lookup to extract tenant_id for the tenant-scoped consumption
    let preview = db
        .get_link_state(state_code)
        .await?
        .ok_or_else(|| AppError::invalid_input("Link code is invalid or expired"))?;

    let tenant_id_str = preview["tenant_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Link state missing tenant_id"))?;
    let tenant_id = TenantId::from_str(tenant_id_str)
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
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);
    let user_id = auth.user_id.to_string();

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
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
    State(resources): State<Arc<ServerResources>>,
    Path(channel): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&auth);
    let user_id = auth.user_id.to_string();

    let channel_type = ChannelType::from_str(&channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown messaging channel: {channel}")))?;

    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
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
    State(resources): State<Arc<ServerResources>>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

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
    State(resources): State<Arc<ServerResources>>,
    Form(form): Form<ChannelLinkAuthForm>,
) -> impl IntoResponse {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

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
    let Ok(tenant_id) = TenantId::from_str(&tenant_id_str) else {
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
        let tenant_repo: &dyn TenantRepository = resources.repos.tenants.as_ref();
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
    resources: &ServerResources,
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
    resources: &ServerResources,
    email: &str,
    password: &str,
) -> Result<Uuid, String> {
    let user_repo: &dyn UserRepository = resources.repos.users.as_ref();

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
    resources: &ServerResources,
    form: &ChannelLinkAuthForm,
) -> Result<Uuid, String> {
    let user_repo: &dyn UserRepository = resources.repos.users.as_ref();

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
