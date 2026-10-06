// ABOUTME: The first-party OAuth2 token endpoint (POST /oauth/token): the password grant and the refresh grant
// ABOUTME: Password sign-ins are refused past their attempt windows (carnet#804) before the password is checked
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::{rejection::FormRejection, Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use tracing::{debug, field, field::Empty, info, Span};

use crate::first_party_client::refuse_unbound_client;
use crate::password_attempts::password_sign_in_refusal;
use crate::token_errors::{grant_error_response, oauth2_error};
use crate::AuthRoutesContext;
use pierre_auth::security::cookies::{set_auth_cookie, set_csrf_cookie};
use pierre_core::errors::AppError;
use pierre_middleware::PeerAddress;

use pierre_auth::dto::auth::{
    LoginRequest, LoginResponse, OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse,
};

use pierre_services::analytics::cache_user_email;
use pierre_services::auth::AuthService;

/// Handle the first-party `OAuth2` token request.
///
/// Two grants. RFC 6749 §4.3, the password grant, is how Dravr's own web and
/// mobile apps sign in, and only them: the request must name one of them as
/// `client_id` (`dravr-web`, `dravr-mobile`), or it is refused with
/// `invalid_client` before the password is checked. Adding
/// `scope=offline_access` asks for a refresh token alongside the JWT. RFC
/// 6749 §6, the refresh grant, exchanges that token for a fresh JWT and a
/// successor token once the JWT has lapsed. It names no client: a refresh
/// token is only ever issued by a password grant, so it is already bound to
/// a first-party client by the grant that issued it.
///
/// Request format: `application/x-www-form-urlencoded`
/// ```text
/// grant_type=password&client_id=dravr-mobile&username=user@example.com&password=secret&scope=offline_access
/// grant_type=refresh_token&refresh_token=...
/// ```
///
/// Response format: RFC 6749 Section 5.1 compliant JSON
#[tracing::instrument(
    skip(resources, peer, headers, request),
    fields(
        route = "oauth2_token",
        grant_type = Empty,
        username = Empty,
        user_id = Empty,
        tenant_id = Empty,
        success = Empty,
    )
)]
pub async fn handle_oauth2_token(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    request: Result<Form<OAuth2TokenRequest>, FormRejection>,
) -> Result<Response, AppError> {
    // RFC 6749 §5.2: a malformed or missing-parameter token request must be
    // answered with an `invalid_request` JSON error, not the framework's bare
    // 422 text body (which also leaks internal field names like `username`).
    let Form(request) = match request {
        Ok(form) => form,
        Err(rejection) => {
            debug!(%rejection, "OAuth2 token request rejected: malformed form body");
            let error_response = OAuth2ErrorResponse {
                error: "invalid_request".to_owned(),
                error_description: Some(
                    "The request is missing a required parameter or is otherwise malformed. \
                     Expected form fields: grant_type, then username and password or \
                     refresh_token."
                        .to_owned(),
                ),
            };
            return Ok((StatusCode::BAD_REQUEST, Json(error_response)).into_response());
        }
    };
    Span::current().record("grant_type", field::display(&request.grant_type));
    if let Some(username) = request.username.as_deref() {
        Span::current().record("username", field::display(&username));
    }

    let auth_service = AuthService::new(
        resources.auth_manager.clone(),
        resources.jwks_manager.clone(),
        resources.config.clone(),
        resources.data.clone(),
    );

    // Each grant resolves to the login-shaped response plus the refresh token
    // the client walks away with, or to the service error the tail maps to
    // an RFC 6749 §5.2 error body.
    let outcome = match request.grant_type.as_str() {
        "password" => {
            // Checked before the password is (carnet#768).
            if let Some(refusal) = refuse_unbound_client(request.client_id.as_deref()) {
                return Ok(refusal);
            }
            let (Some(email), Some(password)) = (request.username, request.password) else {
                return Ok(oauth2_error(
                    "invalid_request",
                    "The password grant needs both username and password.",
                ));
            };
            // The sign-in windows are read before the password is checked, and a
            // refused password is counted in them (carnet#804).
            let limiter = &resources.rate_limiter;
            let client = peer.0.map(|peer| limiter.client_address(peer, &headers));
            if let Some(refusal) = password_sign_in_refusal(limiter, client, &email).await {
                return Ok(refusal);
            }
            let login_request = LoginRequest {
                email,
                password,
                // The password grant's form carries no timezone field.
                timezone: None,
            };
            let attempted = login_request.email.clone();
            match auth_service.login(login_request).await {
                Ok(response) => {
                    let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                        .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;

                    // notify: record tenant/user on the current span so the NotifyLayer can
                    // attribute the user.login event without the call site re-passing IDs.
                    // tenant_id is optional on UserInfo; only record when present so the
                    // routing layer sees an empty field rather than a literal "None".
                    Span::current().record("user_id", field::display(&user_id));
                    if let Some(tenant_id) = response.user.tenant_id.as_deref() {
                        Span::current().record("tenant_id", field::display(&tenant_id));
                    }
                    // Warm the identity cache before emitting so the NotifyLayer enricher
                    // can attach this user's email to the user.login event itself.
                    cache_user_email(&user_id.to_string(), &response.user.email);
                    info!(
                        target: "notify",
                        event = "user.login",
                        "user authenticated"
                    );

                    // A refresh token only for the client that asked for one.
                    // The web app never does: its session is the cookie, and a
                    // token it would discard is a live credential in a table.
                    let refresh_token = if requests_offline_access(request.scope.as_deref()) {
                        Some(
                            auth_service
                                .issue_refresh_token(user_id, response.user.tenant_id.clone())
                                .await?,
                        )
                    } else {
                        None
                    };
                    Ok((response, refresh_token))
                }
                Err(e) => {
                    if !e.is_server_fault() {
                        limiter.count_failed_sign_in(client, &attempted).await;
                    }
                    Err(e)
                }
            }
        }
        "refresh_token" => {
            let Some(presented) = request.refresh_token else {
                return Ok(oauth2_error(
                    "invalid_request",
                    "The refresh_token grant needs refresh_token.",
                ));
            };
            auth_service
                .refresh_session(&presented)
                .await
                .map(|session| (session.login, Some(session.refresh_token)))
        }
        other => {
            return Ok(oauth2_error(
                "unsupported_grant_type",
                &format!(
                    "Grant type '{other}' is not supported. Use 'password' or 'refresh_token'."
                ),
            ));
        }
    };

    match outcome {
        Ok((response, refresh_token)) => {
            issue_tokens(&resources, response, refresh_token, request.scope)
        }
        Err(e) => Ok(grant_error_response(e)),
    }
}

/// The scope a client requests to receive a refresh token alongside its JWT.
///
/// `OpenID` Connect Core §11's name for "I will need to act while the user is
/// away", which is exactly what a phone closed for a week is.
const OFFLINE_ACCESS_SCOPE: &str = "offline_access";

/// Whether a password grant asked for a refresh token.
fn requests_offline_access(scope: Option<&str>) -> bool {
    scope.is_some_and(|scope| {
        scope
            .split_ascii_whitespace()
            .any(|part| part == OFFLINE_ACCESS_SCOPE)
    })
}

/// Turn a login-shaped response into the RFC 6749 §5.1 token response, with
/// the CSRF token and cookies a web client needs. The password and refresh
/// grants both end here, so a session minted either way looks the same.
fn issue_tokens(
    resources: &AuthRoutesContext,
    response: LoginResponse,
    refresh_token: Option<String>,
    scope: Option<String>,
) -> Result<Response, AppError> {
    let jwt_token = response
        .jwt_token
        .ok_or_else(|| AppError::internal("JWT token missing from login response"))?;

    // Parse expiration to calculate expires_in
    let expires_at = chrono::DateTime::parse_from_rfc3339(&response.expires_at).map_or_else(
        |_| chrono::Utc::now() + chrono::Duration::hours(24),
        |dt| dt.with_timezone(&chrono::Utc),
    );
    let expires_in = (expires_at - chrono::Utc::now()).num_seconds();

    // Generate CSRF token for web clients (stateless HMAC — no server storage)
    let user_id = uuid::Uuid::parse_str(&response.user.user_id)
        .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;
    let csrf_token = resources
        .csrf_manager
        .generate_token(user_id)
        .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

    // Build response with secure cookies for web clients
    let mut headers = HeaderMap::new();
    set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);
    set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

    let oauth2_response = OAuth2TokenResponse {
        access_token: jwt_token,
        token_type: "Bearer".to_owned(),
        expires_in,
        refresh_token,
        scope,
        // Pierre extensions for frontend compatibility
        user: Some(response.user),
        csrf_token: Some(csrf_token),
    };

    Span::current().record("success", true);
    Ok((StatusCode::OK, headers, Json(oauth2_response)).into_response())
}
