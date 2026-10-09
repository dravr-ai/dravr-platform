// ABOUTME: The first-party OAuth2 token endpoint (POST /oauth/token): the authorization-code grant and the refresh grant
// ABOUTME: Dravr's own apps redeem the code the hosted login page issued, with PKCE, for a first-party session
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

use crate::token_errors::{grant_error_response, oauth2_error};
use crate::AuthRoutesContext;
use pierre_auth::oauth2_server::endpoints::OAuth2AuthorizationServer;
use pierre_auth::oauth2_server::first_party::{
    MOBILE_APP_ATTEST_APP_ID, MOBILE_CLIENT_ID, MOBILE_PLAY_PACKAGE_NAME,
};
use pierre_auth::oauth2_server::mobile_attestation::{
    record_evidence, verify_evidence, verify_play_integrity, AppAttestEvidence, MobileEvidence,
    PlayIntegrityOutcome, VerifiedEvidence,
};
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::security::cookies::{set_auth_cookie, set_csrf_cookie};
use pierre_core::errors::AppError;

use pierre_auth::dto::auth::{
    LoginResponse, OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse,
};

use pierre_services::analytics::cache_user_email;
use pierre_services::auth::AuthService;

/// Handle the first-party `OAuth2` token request.
///
/// Two grants, both for Dravr's own web and mobile apps. RFC 6749 §4.1.3,
/// the authorization-code grant, is how they sign in (carnet#787): the
/// athlete typed the password on the hosted login page, never into the app,
/// and the app redeems the code it was issued with its PKCE verifier. The
/// request names the app as `client_id` (`dravr-web`, `dravr-mobile`), the
/// `redirect_uri` the code was sent to, and `code_verifier`; adding
/// `scope=offline_access` asks for a refresh token alongside the JWT. RFC
/// 6749 §6, the refresh grant, exchanges that token for a fresh JWT and a
/// successor token once the JWT has lapsed. It names no client: a refresh
/// token is only ever issued here, so it is already bound to a first-party
/// client by the sign-in that issued it.
///
/// The iOS app adds App Attest evidence to its code exchange (carnet#810):
/// `app_attest_key_id` with `app_attest_attestation` on an install's first
/// sign-in, or `app_attest_assertion` on every later one, both signed over
/// the code. The Android app sends `play_integrity_token` instead: a Play
/// Integrity token requested over the base64url (unpadded) SHA-256 of the
/// code, which Google decodes. Evidence that is present must verify; see
/// [`check_device_evidence`].
///
/// The password grant (RFC 6749 §4.3) is not served: RFC 9700 §2.4 forbids
/// it, and it let anything holding a password become Dravr's own app.
///
/// Request format: `application/x-www-form-urlencoded`
/// ```text
/// grant_type=authorization_code&client_id=dravr-mobile&code=...&redirect_uri=dravr%3A%2F%2Fauth%2Fcallback&code_verifier=...&scope=offline_access
/// grant_type=refresh_token&refresh_token=...
/// ```
///
/// Response format: RFC 6749 Section 5.1 compliant JSON
#[tracing::instrument(
    skip(resources, request),
    fields(
        route = "oauth2_token",
        grant_type = Empty,
        client_id = Empty,
        user_id = Empty,
        tenant_id = Empty,
        app_attest = Empty,
        success = Empty,
    )
)]
pub async fn handle_oauth2_token(
    State(resources): State<AuthRoutesContext>,
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
                     Expected form fields: grant_type, then client_id, code, redirect_uri \
                     and code_verifier, or refresh_token."
                        .to_owned(),
                ),
            };
            return Ok((StatusCode::BAD_REQUEST, Json(error_response)).into_response());
        }
    };
    Span::current().record("grant_type", field::display(&request.grant_type));
    if let Some(client_id) = request.client_id.as_deref() {
        Span::current().record("client_id", field::display(&client_id));
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
        "authorization_code" => {
            let (Some(client_id), Some(code), Some(redirect_uri)) = (
                request.client_id.as_deref(),
                request.code.as_deref(),
                request.redirect_uri.as_deref(),
            ) else {
                return Ok(oauth2_error(
                    "invalid_request",
                    "The authorization_code grant needs client_id, code, redirect_uri and \
                     code_verifier.",
                ));
            };
            let attested =
                match check_device_evidence(&resources, &request, client_id, code, redirect_uri)
                    .await
                {
                    Ok(attested) => attested,
                    Err(refusal) => return Ok(refused_evidence(&refusal)),
                };
            let redeemed = match authorization_server(&resources)
                .redeem_first_party_code(
                    client_id,
                    code,
                    redirect_uri,
                    request.code_verifier.as_deref(),
                )
                .await
            {
                Ok(redeemed) => redeemed,
                Err(refusal) => return Ok(refused_code(&refusal)),
            };
            if let Some(verified) = attested {
                if let Err(refusal) = record_evidence(
                    resources.repos.app_attest_keys.as_ref(),
                    verified,
                    chrono::Utc::now(),
                )
                .await
                {
                    return Ok(refused_evidence(&refusal));
                }
            }
            match auth_service
                .sign_in_with_code(redeemed.user_id, &redeemed.tenant_id)
                .await
            {
                Err(e) => Err(e),
                Ok(response) => {
                    record_sign_in(&response);
                    // A refresh token only for the client that asked for one.
                    // The web app never does: its session is the cookie, and a
                    // token it would discard is a live credential in a table.
                    let refresh_token = if requests_offline_access(request.scope.as_deref()) {
                        Some(
                            auth_service
                                .issue_refresh_token(
                                    redeemed.user_id,
                                    response.user.tenant_id.clone(),
                                )
                                .await?,
                        )
                    } else {
                        None
                    };
                    Ok((response, refresh_token))
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
                    "Grant type '{other}' is not supported. Use 'authorization_code' or \
                     'refresh_token'."
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

/// Whether a sign-in asked for a refresh token.
fn requests_offline_access(scope: Option<&str>) -> bool {
    scope.is_some_and(|scope| {
        scope
            .split_ascii_whitespace()
            .any(|part| part == OFFLINE_ACCESS_SCOPE)
    })
}

/// Turn a login-shaped response into the RFC 6749 §5.1 token response, with
/// the CSRF token and cookies a web client needs. The code and refresh
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

/// The authorization server that issued the code, serving this deployment's
/// first-party redirects.
fn authorization_server(resources: &AuthRoutesContext) -> OAuth2AuthorizationServer {
    let repos = &resources.repos;
    let oauth2 = &resources.config.oauth2_server;
    OAuth2AuthorizationServer::new(
        repos.oauth2_server.clone(),
        repos.tenants.clone(),
        repos.users.clone(),
        resources.auth_manager.clone(),
        resources.jwks_manager.clone(),
        resources.config.auth.refresh_token_expiry_days,
        oauth2
            .mcp_resources()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
    .with_first_party_redirects(oauth2.first_party_redirects.clone())
}

/// Verify the device evidence a code exchange carried, before the code is
/// redeemed (carnet#810): App Attest from the iOS app, Play Integrity from
/// the Android app. A Play Integrity token is only sent to Google for a code
/// that would redeem (live, this client and redirect, PKCE passing).
///
/// Only Dravr's mobile app attests, so evidence from any other client is a
/// malformed request. Verified App Attest evidence comes back to be recorded
/// once the code is redeemed; a verified Play Integrity verdict leaves
/// nothing to record. A refusal leaves the code unspent, so the app can
/// retry with fresh evidence. The span records which kind of evidence the
/// mobile sign-in carried, `absent` included: absence is accepted until
/// enforcement is switched on, and the span is where its share is read.
async fn check_device_evidence(
    resources: &AuthRoutesContext,
    request: &OAuth2TokenRequest,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<Option<VerifiedEvidence>, OAuth2Error> {
    let evidence = MobileEvidence::from_request(
        request.app_attest_key_id.as_deref(),
        request.app_attest_attestation.as_deref(),
        request.app_attest_assertion.as_deref(),
        request.play_integrity_token.as_deref(),
    )
    .inspect_err(|_| {
        if client_id == MOBILE_CLIENT_ID {
            Span::current().record("app_attest", "refused");
        }
    })?;
    if client_id != MOBILE_CLIENT_ID {
        return match evidence {
            None => Ok(None),
            Some(_) => Err(OAuth2Error::invalid_request(
                "Device attestation evidence is accepted from dravr-mobile only.",
            )),
        };
    }
    match evidence {
        None => {
            Span::current().record("app_attest", "absent");
            Ok(None)
        }
        Some(MobileEvidence::AppAttest(evidence)) => {
            check_app_attest_evidence(resources, &evidence, code)
                .await
                .map(Some)
        }
        Some(MobileEvidence::PlayIntegrity(token)) => {
            // Each decode is a call to Google, metered against a daily quota:
            // only a code the exchange could redeem is worth one. Any other
            // code goes on to the redemption, which refuses it as it would
            // without evidence.
            let redeemable = authorization_server(resources)
                .check_first_party_code(
                    client_id,
                    code,
                    redirect_uri,
                    request.code_verifier.as_deref(),
                )
                .await;
            if let Err(refusal) = redeemable {
                debug!(
                    error = %refusal.error,
                    "Play Integrity token not decoded: the code would not redeem"
                );
                return Ok(None);
            }
            check_play_integrity(resources, &token, code).await?;
            Ok(None)
        }
    }
}

/// Verify the iOS app's App Attest evidence over `code`, recording its kind
/// on the span.
async fn check_app_attest_evidence(
    resources: &AuthRoutesContext,
    evidence: &AppAttestEvidence,
    code: &str,
) -> Result<VerifiedEvidence, OAuth2Error> {
    let verified = verify_evidence(
        resources.repos.app_attest_keys.as_ref(),
        evidence,
        code,
        MOBILE_APP_ATTEST_APP_ID,
        chrono::Utc::now(),
    )
    .await
    .inspect_err(|_| {
        Span::current().record("app_attest", "refused");
    })?;
    Span::current().record("app_attest", verified.kind());
    Ok(verified)
}

/// Have Google decode the Android app's Play Integrity token and check the
/// verdict against `code`, recording the outcome on the span. Google out of
/// reach is `unavailable` and lets the exchange proceed, as absent evidence
/// does while enforcement is off.
async fn check_play_integrity(
    resources: &AuthRoutesContext,
    token: &str,
    code: &str,
) -> Result<(), OAuth2Error> {
    let outcome = verify_play_integrity(
        resources.play_integrity.as_ref(),
        token,
        code,
        MOBILE_PLAY_PACKAGE_NAME,
        chrono::Utc::now(),
    )
    .await
    .inspect_err(|_| {
        Span::current().record("app_attest", "refused");
    })?;
    Span::current().record("app_attest", outcome.kind());
    if let PlayIntegrityOutcome::Verified(verdict) = outcome {
        info!(
            strong_integrity = verdict.strong_integrity,
            "Play Integrity verdict verified"
        );
    }
    Ok(())
}

/// The answer to device attestation evidence that was refused or malformed:
/// the error's own status, so `invalid_client` is a 400. RFC 6749 §5.2 asks for
/// a 401 only when the client authenticated through the `Authorization`
/// header, which evidence never travels in, and the apps read any 401 as a
/// session that lapsed — clearing it and announcing a sign-out in the middle
/// of a sign-in.
fn refused_evidence(refusal: &OAuth2Error) -> Response {
    let body = OAuth2ErrorResponse {
        error: refusal.error.clone(),
        error_description: refusal.error_description.clone(),
    };
    (refusal.http_status(), Json(body)).into_response()
}

/// The RFC 6749 §5.2 answer to a code the authorization server refused:
/// 401 for `invalid_client`, 500 for its own fault, 400 otherwise.
fn refused_code(refusal: &OAuth2Error) -> Response {
    let status = match refusal.error.as_str() {
        "invalid_client" => StatusCode::UNAUTHORIZED,
        "server_error" => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::BAD_REQUEST,
    };
    let body = OAuth2ErrorResponse {
        error: refusal.error.clone(),
        error_description: refusal.error_description.clone(),
    };
    (status, Json(body)).into_response()
}

/// Record a completed sign-in on the span and raise the `user.login` notify
/// event for it.
fn record_sign_in(response: &LoginResponse) {
    // notify: record tenant/user on the current span so the NotifyLayer can
    // attribute the user.login event without the call site re-passing IDs.
    // tenant_id is optional on UserInfo; only record when present so the
    // routing layer sees an empty field rather than a literal "None".
    Span::current().record("user_id", field::display(&response.user.user_id));
    if let Some(tenant_id) = response.user.tenant_id.as_deref() {
        Span::current().record("tenant_id", field::display(&tenant_id));
    }
    // Warm the identity cache before emitting so the NotifyLayer enricher
    // can attach this user's email to the user.login event itself.
    cache_user_email(&response.user.user_id, &response.user.email);
    info!(
        target: "notify",
        event = "user.login",
        "user authenticated"
    );
}
