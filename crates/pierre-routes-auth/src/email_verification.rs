// ABOUTME: Email-verification endpoints — issue a link, consume it, re-send it
// ABOUTME: Split from login.rs: proving an address is its own concern, not part of signing in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Confirming that a registered address belongs to the person who typed it.
//!
//! Three surfaces, all anti-enumeration by construction:
//!
//! - [`issue_verification_email`] — mints a single-use token and mails it.
//!   Best-effort: registration has already succeeded by the time it runs.
//! - [`handle_verify_email`] — consumes the token from the mailed link, stamps
//!   the address as proven, and applies the approval decision.
//! - [`handle_resend_verification`] — re-issues a link, answering identically
//!   whether or not the address exists.
//!
//! These lived in `login.rs` until that file crossed the size ratchet. The
//! split is not bookkeeping: verifying an address is a distinct lifecycle step
//! from authenticating a session, and the only coupling left is registration
//! calling [`issue_verification_email`] once.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::json;
use tracing::{error, info, warn};

use crate::AuthRoutesContext;
use pierre_core::errors::AppError;
use pierre_core::models::UserStatus;
use pierre_services::auth::AuthService;
use pierre_services::email_verification::resolve_settings;
use pierre_services::link_token::{generate_link_token, split_link_token};

/// Issue a verification link for `user_id` and email it.
///
/// Best-effort throughout: registration has already succeeded by the time this
/// runs, so a mail failure must never turn a created account into an error
/// response. The user can always ask for another link from the waiting screen.
///
/// Returns `true` when a link was actually sent, so callers can tell the client
/// whether to expect mail.
pub async fn issue_verification_email(
    resources: &AuthRoutesContext,
    user_id: uuid::Uuid,
    email: &str,
    display_name: Option<&str>,
) -> bool {
    let settings = resolve_settings(resources.data.database().as_ref()).await;

    if !within_send_budget(resources, user_id, settings.max_sends_per_hour).await {
        return false;
    }

    let Some(verify_url) = mint_verification_url(resources, user_id, settings.ttl_minutes).await
    else {
        return false;
    };

    let sent = deliver(
        resources,
        email,
        display_name,
        &verify_url,
        settings.ttl_minutes,
    )
    .await;
    if sent {
        info!(user_id = %user_id, "verification email sent");
    }
    sent
}

/// Hand the link to the mail service.
///
/// Separated from the caller so each step of issuing a link — budget, mint,
/// deliver — fails in one place and reads as one decision.
async fn deliver(
    resources: &AuthRoutesContext,
    email: &str,
    display_name: Option<&str>,
    verify_url: &str,
    ttl_minutes: i64,
) -> bool {
    let Some(email_svc) = &resources.email_service else {
        warn!("email service not configured — verification link generated but not delivered");
        return false;
    };

    if let Err(e) = email_svc
        .send_email_verification(email, display_name, verify_url, ttl_minutes)
        .await
    {
        warn!(error = %e, "failed to send verification email");
        return false;
    }

    true
}

/// Whether this user may be sent another link within the hourly cap.
///
/// A caller that trips the cap is answered exactly like a caller that didn't,
/// so the endpoint stays anti-enumeration. A failure to *count* sends anyway:
/// silently withholding the link over a transient database error would strand
/// someone mid-signup, which is worse than one extra email.
async fn within_send_budget(
    resources: &AuthRoutesContext,
    user_id: uuid::Uuid,
    max_per_hour: i64,
) -> bool {
    let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    match resources
        .repos
        .email_verification
        .count_recent_tokens(user_id, one_hour_ago)
        .await
    {
        Ok(count) if count >= max_per_hour => {
            info!(
                user_id = %user_id,
                "verification email rate limit reached — not sending"
            );
            false
        }
        Ok(_) => true,
        Err(e) => {
            warn!(error = %e, "failed to count verification tokens; sending anyway");
            true
        }
    }
}

/// Mint and persist a single-use token, returning the link to mail.
///
/// The link points at the API, not the SPA: the token is consumed server-side
/// and the browser is then redirected to the frontend with a result code, so a
/// single-use token never sits in the SPA's history or a referrer header.
///
/// `None` means the token could not be stored, so no link may be sent — mailing
/// one that will never verify is worse than mailing nothing.
async fn mint_verification_url(
    resources: &AuthRoutesContext,
    user_id: uuid::Uuid,
    ttl_minutes: i64,
) -> Option<String> {
    let generated = generate_link_token();
    if let Err(e) = resources
        .repos
        .email_verification
        .store_token(
            user_id,
            &generated.selector,
            &generated.verifier_hash,
            ttl_minutes,
        )
        .await
    {
        error!(error = %e, "failed to store verification token — no link sent");
        return None;
    }

    Some(format!(
        "{}/api/auth/verify-email?token={}",
        resources.config.base_url,
        urlencoding::encode(&generated.token)
    ))
}

/// Query string for `GET /api/auth/verify-email`.
#[derive(Debug, serde::Deserialize)]
pub struct VerifyEmailQuery {
    /// The `<selector>.<verifier>` token from the emailed link.
    pub token: Option<String>,
}

/// Where to send the browser after a verification attempt.
///
/// Always a redirect, never JSON: this URL is opened by a human clicking a link
/// in their mail client, so the response has to be a page they can read.
fn verification_redirect(resources: &AuthRoutesContext, status: &str) -> Response {
    let base = resources
        .config
        .frontend_url
        .as_deref()
        .unwrap_or(&resources.config.base_url);
    let location = format!("{base}/verify-email?status={status}");
    Redirect::to(&location).into_response()
}

/// Split the `<selector>.<verifier>` token and claim it, returning the user it
/// proves.
///
/// Every rejection reason collapses to `None` deliberately: a missing token, a
/// malformed one, an expired one and a wrong verifier are all answered with the
/// same redirect, so the endpoint never tells a caller which of those it was.
async fn claim_token(resources: &AuthRoutesContext, token: Option<&str>) -> Option<uuid::Uuid> {
    let token = token.filter(|t| !t.is_empty())?;
    let (selector, verifier_hash) = split_link_token(token)?;

    match resources
        .repos
        .email_verification
        .consume_token(&selector, &verifier_hash)
        .await
    {
        Ok(id) => Some(id),
        Err(e) => {
            info!(error = %e, "verification token rejected");
            None
        }
    }
}

/// `GET /api/auth/verify-email?token=...`
///
/// Consumes the emailed token, stamps the address as proven, and applies the
/// approval decision — which promotes the account to Active when auto-approval
/// says so, and otherwise leaves it Pending but verified.
///
/// # Errors
///
/// Never returns `Err`: every outcome is a redirect carrying a status the SPA
/// renders, because the caller is a mail client following a link and cannot do
/// anything useful with a JSON error body.
pub async fn handle_verify_email(
    State(resources): State<AuthRoutesContext>,
    Query(query): Query<VerifyEmailQuery>,
) -> Response {
    let Some(user_id) = claim_token(&resources, query.token.as_deref()).await else {
        return verification_redirect(&resources, "invalid");
    };

    if let Err(e) = resources
        .repos
        .email_verification
        .mark_verified(user_id)
        .await
    {
        error!(error = %e, user_id = %user_id, "failed to stamp email as verified");
        return verification_redirect(&resources, "error");
    }

    let auth_service = AuthService::new(
        resources.auth_manager.clone(),
        resources.jwks_manager.clone(),
        resources.config.clone(),
        resources.data.clone(),
    );

    let status = match auth_service
        .apply_approval_after_verification(user_id)
        .await
    {
        Ok(status) => status,
        Err(e) => {
            // The address is proven and stamped; only the promotion failed. Send
            // the user on as verified — their next login re-runs the same
            // decision, so this self-heals rather than stranding them.
            warn!(error = %e, user_id = %user_id, "approval decision failed after verification");
            UserStatus::Pending
        }
    };

    // No `notify` event here on purpose. `user.email_verified` would be a
    // genuinely useful funnel step — signup → verified → connected — but the
    // event catalogue lives in dravr-contremaitre, so adding one means a
    // cross-repo change plus a rev bump before the platform can emit it.
    // The fact itself is durable in `users.email_verified_at`, so nothing is
    // lost by adding the event when contremaitre is next touched.
    // Two different destinations because they are two different situations: an
    // active account should go and sign in, a pending one is still waiting on a
    // human and needs to be told that rather than bounced at a login form.
    let outcome = if status == UserStatus::Active {
        "verified"
    } else {
        "verified_pending"
    };
    verification_redirect(&resources, outcome)
}

/// Request body for `POST /api/auth/resend-verification`.
#[derive(Debug, serde::Deserialize)]
pub struct ResendVerificationRequest {
    /// Address to re-send the confirmation link to.
    pub email: String,
}

/// `POST /api/auth/resend-verification`
///
/// Re-issues a confirmation link. Always answers with the same message whether
/// or not the address exists, is already verified, or has tripped its hourly
/// cap — the response must not let a caller probe which addresses are registered.
///
/// # Errors
///
/// Returns `AppError` only for a malformed email; every account-state outcome is
/// a 200 with the neutral message.
pub async fn handle_resend_verification(
    State(resources): State<AuthRoutesContext>,
    Json(request): Json<ResendVerificationRequest>,
) -> Result<Response, AppError> {
    let neutral = json!({
        "message": "If that address needs confirming, we've sent a new link."
    });

    if !request.email.contains('@') || request.email.len() < 5 {
        return Err(AppError::invalid_input("Invalid email format"));
    }

    let Ok(Some(user)) = resources.repos.users.get_by_email(&request.email).await else {
        info!("resend-verification for unknown address (anti-enumeration)");
        return Ok((StatusCode::OK, Json(neutral)).into_response());
    };

    // Already proven — silently do nothing rather than mail a pointless link.
    if resources
        .repos
        .email_verification
        .is_verified(user.id)
        .await
        .unwrap_or(false)
    {
        return Ok((StatusCode::OK, Json(neutral)).into_response());
    }

    issue_verification_email(
        &resources,
        user.id,
        &request.email,
        user.display_name.as_deref(),
    )
    .await;

    Ok((StatusCode::OK, Json(neutral)).into_response())
}
