// ABOUTME: Email-verification endpoints — issue a link, confirm it on a button press, re-send it
// ABOUTME: Split from login.rs: proving an address is its own concern, not part of signing in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Confirming that a registered address belongs to the person who typed it.
//!
//! Four surfaces, all anti-enumeration by construction:
//!
//! - [`issue_verification_email`] — mints a single-use token and mails a link
//!   to the web app's confirmation page. Best-effort: registration has already
//!   succeeded by the time it runs.
//! - [`handle_verify_email_link`] — a GET on the API's verify URL (the shape
//!   links had before the page did): forwards to the page and claims nothing.
//! - [`handle_confirm_email`] — the page's button: claims the token, stamps the
//!   address as proven, and applies the approval decision.
//! - [`handle_resend_verification`] — re-issues a link for an address, or for
//!   a link that no longer verifies, answering identically whether or not the
//!   account exists.
//!
//! Claiming only on a POST is the point. Mail security scanners (Outlook Safe
//! Links, corporate gateways) and prefetching clients GET every link in a
//! message; when the GET claimed the token, the person's own click found it
//! spent and was told the link had expired, on an address that was in fact
//! proven.
//!
//! These lived in `login.rs` until that file crossed the size ratchet. The
//! split is not bookkeeping: verifying an address is a distinct lifecycle step
//! from authenticating a session, and the only coupling left is registration
//! calling [`issue_verification_email`] once.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use tracing::{error, info, warn};

use crate::signup_guards::{client_address, meter_sign_up};
use crate::AuthRoutesContext;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{User, UserStatus};
use pierre_middleware::PeerAddress;
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
pub(crate) async fn issue_verification_email(
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
/// The link opens the web app's confirmation page with the token in the
/// fragment: a fragment never reaches a server log or a referrer header, and
/// the page claims the token only when the person presses its button. A mail
/// scanner that follows every link therefore cannot spend it first.
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

    Some(confirmation_page_url(resources, &generated.token))
}

/// The web app's confirmation page, carrying `token` in its fragment.
fn confirmation_page_url(resources: &AuthRoutesContext, token: &str) -> String {
    let base = resources
        .config
        .frontend_url
        .as_deref()
        .unwrap_or(&resources.config.base_url);
    format!(
        "{base}{VERIFY_EMAIL_PAGE}#token={}",
        urlencoding::encode(token)
    )
}

/// The web app's page a confirmation link opens.
const VERIFY_EMAIL_PAGE: &str = "/verify-email";

/// Query string for `GET /api/auth/verify-email`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct VerifyEmailQuery {
    /// The `<selector>.<verifier>` token from the link.
    pub token: Option<SecretString>,
}

/// `GET /api/auth/verify-email?token=...`
///
/// Sends the browser on to the confirmation page with the token moved into the
/// fragment, and claims nothing. A GET is what every link scanner, preview
/// fetcher and prefetching mail client issues on its own, so it must never
/// spend a single-use credential; only the page's button does that, through
/// [`handle_confirm_email`].
pub(crate) async fn handle_verify_email_link(
    State(resources): State<AuthRoutesContext>,
    Query(query): Query<VerifyEmailQuery>,
) -> Response {
    let token = query.token.as_ref().map_or("", ExposeSecret::expose_secret);
    Redirect::to(&confirmation_page_url(&resources, token)).into_response()
}

/// Body of `POST /api/auth/verify-email`.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ConfirmEmailRequest {
    /// The `<selector>.<verifier>` token from the link.
    pub token: SecretString,
}

/// What a confirmation attempt came to, as the page renders it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConfirmOutcome {
    /// The address is proven and the account is active: go and sign in
    Verified,
    /// The address is proven but a human still has to approve the account
    VerifiedPending,
    /// The link is genuine but no longer verifies and the address is still
    /// unproven: the page offers a fresh link for this same token
    Expired,
    /// Not a link this server issued, or a wrong verifier
    Invalid,
}

/// `POST /api/auth/verify-email` with `{ "token": … }`
///
/// Claims the token, stamps the address as proven, and applies the approval
/// decision — which promotes the account to Active when auto-approval says so,
/// and otherwise leaves it Pending but verified.
///
/// A token that no longer verifies is not the end of it. When the caller holds
/// the whole token of a link this server issued, the answer is about that
/// account: a second press, or a link a scanner already followed, finds the
/// address proven and answers exactly as the first press did; an expired link
/// on an unproven address answers `expired`, and the page re-sends from the
/// token. Only the holder of the matching verifier learns any of this, so the
/// endpoint still reveals nothing about addresses to anyone else.
///
/// Counted in the client address's sign-up window, as the resend is, so one
/// address cannot press through tokens faster than a person would.
///
/// # Errors
///
/// Returns `AppError` for a request past its address's window and when the
/// database cannot be read or written; every state a link can be in is a 200
/// carrying its [`ConfirmOutcome`].
pub(crate) async fn handle_confirm_email(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    Json(request): Json<ConfirmEmailRequest>,
) -> Result<Response, AppError> {
    let client = client_address(&resources.rate_limiter, peer, &headers);
    meter_sign_up(&resources.rate_limiter, client).await?;
    let outcome = confirm(&resources, request.token.expose_secret()).await?;
    Ok((StatusCode::OK, Json(json!({ "status": outcome }))).into_response())
}

/// The confirmation itself, apart from its HTTP shape.
async fn confirm(resources: &AuthRoutesContext, token: &str) -> Result<ConfirmOutcome, AppError> {
    let Some((selector, verifier_hash)) = split_link_token(token) else {
        return Ok(ConfirmOutcome::Invalid);
    };
    let repo = &resources.repos.email_verification;

    let user_id = match repo.consume_token(&selector, &verifier_hash).await {
        Ok(user_id) => user_id,
        Err(e) if e.is_server_fault() => return Err(e),
        Err(e) => {
            info!(error = %e, "verification token refused; checking whose link it is");
            return spent_link_outcome(resources, &selector, &verifier_hash).await;
        }
    };

    let first_proof = repo.mark_verified(user_id).await?;
    let status = match resources
        .account_service()
        .apply_approval_after_verification(user_id, first_proof)
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
    Ok(proven(status))
}

/// The outcome for a token `consume_token` refused: used, expired, locked out,
/// or never issued.
async fn spent_link_outcome(
    resources: &AuthRoutesContext,
    selector: &str,
    verifier_hash: &str,
) -> Result<ConfirmOutcome, AppError> {
    let repo = &resources.repos.email_verification;
    let Some(user_id) = repo.token_owner(selector, verifier_hash).await? else {
        info!("verification token rejected");
        return Ok(ConfirmOutcome::Invalid);
    };
    if !repo.is_verified(user_id).await? {
        info!(user_id = %user_id, "verification link no longer valid; address unproven");
        return Ok(ConfirmOutcome::Expired);
    }
    let status = resources
        .repos
        .users
        .get_global(user_id)
        .await?
        .map_or(UserStatus::Pending, |user| user.user_status);
    Ok(proven(status))
}

/// Where a proven address sends its owner. Two destinations because they are
/// two situations: an active account should go and sign in, a pending one is
/// still waiting on a human and needs to be told that rather than bounced at a
/// login form.
fn proven(status: UserStatus) -> ConfirmOutcome {
    if status == UserStatus::Active {
        ConfirmOutcome::Verified
    } else {
        ConfirmOutcome::VerifiedPending
    }
}

/// Request body for `POST /api/auth/resend-verification`: exactly one of the
/// two ways to name the account.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct ResendVerificationRequest {
    /// Address to re-send the confirmation link to — the waiting screen of a
    /// signed-in account, which knows it.
    pub email: Option<String>,
    /// A link this server issued that no longer verifies — the confirmation
    /// page, which holds it and must not ask the person to type their address.
    pub token: Option<SecretString>,
}

/// `POST /api/auth/resend-verification`
///
/// Re-issues a confirmation link. Always answers with the same message whether
/// or not the account exists, is already verified, or has tripped its hourly
/// cap — the response must not let a caller probe which addresses are registered.
///
/// Counted in the client address's sign-up window (carnet#1063), so one
/// address cannot spread resends across many accounts' hourly caps.
///
/// # Errors
///
/// Returns `AppError` for a request past its address's window, and for a
/// body that names the account by neither or both of `email` and `token`, or
/// by a malformed email; every account-state outcome is a 200 with the
/// neutral message.
pub(crate) async fn handle_resend_verification(
    State(resources): State<AuthRoutesContext>,
    peer: PeerAddress,
    headers: HeaderMap,
    Json(request): Json<ResendVerificationRequest>,
) -> Result<Response, AppError> {
    let neutral = json!({
        "message": "If that address needs confirming, we've sent a new link."
    });

    let client = client_address(&resources.rate_limiter, peer, &headers);
    meter_sign_up(&resources.rate_limiter, client).await?;

    let user = match (request.email, request.token) {
        (Some(email), None) => {
            if !email.contains('@') || email.len() < 5 {
                return Err(AppError::invalid_input("Invalid email format"));
            }
            resources.repos.users.get_by_email(&email).await
        }
        (None, Some(token)) => link_owner(&resources, token.expose_secret()).await,
        (None, None) => {
            return Err(AppError::invalid_input(
                "Name the account by email or by link",
            ))
        }
        (Some(_), Some(_)) => {
            return Err(AppError::invalid_input(
                "Name the account by email or by link, not both",
            ))
        }
    };
    let user = match user {
        Ok(Some(user)) => user,
        Ok(None) => {
            info!("resend-verification for unknown account (anti-enumeration)");
            return Ok((StatusCode::OK, Json(neutral)).into_response());
        }
        Err(e) => {
            // The caller still gets the neutral answer; only the log knows the
            // lookup failed rather than finding no account.
            error!(error = %e, "resend-verification could not look up the account");
            return Ok((StatusCode::OK, Json(neutral)).into_response());
        }
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
        &user.email,
        user.display_name.as_deref(),
    )
    .await;

    Ok((StatusCode::OK, Json(neutral)).into_response())
}

/// The account a link was issued to, when the caller holds its whole token.
async fn link_owner(resources: &AuthRoutesContext, token: &str) -> AppResult<Option<User>> {
    let Some((selector, verifier_hash)) = split_link_token(token) else {
        return Ok(None);
    };
    match resources
        .repos
        .email_verification
        .token_owner(&selector, &verifier_hash)
        .await?
    {
        Some(user_id) => resources.repos.users.get_global(user_id).await,
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::VERIFY_EMAIL_PAGE;

    /// The web app's side of the link, relative to this crate.
    const WEB_TOKEN_LINK: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../frontend/src/utils/tokenLink.ts"
    );

    /// A parity check between two artefacts nothing at runtime joins: this
    /// crate builds the confirmation link, the web app routes its path and
    /// reads the `token` fragment key [`super::confirmation_page_url`] writes.
    #[test]
    fn the_web_app_routes_the_confirmation_path_and_reads_its_fragment() {
        let web = fs::read_to_string(WEB_TOKEN_LINK).expect("the web app's link module");
        assert!(
            web.contains(&format!(
                "export const VERIFY_EMAIL_PATH = '{VERIFY_EMAIL_PAGE}';"
            )),
            "the web app must route {VERIFY_EMAIL_PAGE}"
        );
        assert!(
            web.contains(".get('token')"),
            "the web app must read the `token` fragment key the link carries"
        );
    }
}
