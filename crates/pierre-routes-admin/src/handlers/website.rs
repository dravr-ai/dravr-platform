// ABOUTME: dravr.ai website endpoints for its Cloudflare Worker — the docs magic link's send and redemption
// ABOUTME: Admin-token routes gated by ManageWebsite; every answer body is uniform so none says who has an account

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What the dravr.ai Worker asks of the platform.
//!
//! The website keeps no user store of its own: the members part of its docs
//! opens to anyone with an active Dravr account, proven by a single-use link
//! mailed to that account's address. The Worker holds an admin token carrying
//! only [`AdminPermission::ManageWebsite`] and calls two routes:
//!
//! - [`handle_send_sign_in_link`] mails a link when the address belongs to an
//!   active account and the account is within its send budget.
//! - [`handle_consume_sign_in_link`] redeems a link once and names the account.
//!
//! The send route answers the same `{"status":"ok"}` for every valid request,
//! whether or not anything was sent, and redemption answers one uniform 404
//! for every failure: the Worker relays what it is told to a visitor, so no
//! response body may tell a visitor whether an address has an account.

use std::sync::Arc;

use axum::{extract::State, Extension, Json};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{error, info, warn};
use uuid::Uuid;

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, User, UserStatus};
use pierre_database::repositories::website_sign_in_tokens::invalid_sign_in_token;
use pierre_services::auth::AuthService;
use pierre_services::link_token::{generate_link_token, split_link_token};

use crate::context::AdminApiContext;

/// How long a docs sign-in link stays redeemable.
const SIGN_IN_LINK_TTL_MINUTES: i64 = 15;

/// Links one account may be sent in a rolling hour, counted from the token table.
const SIGN_IN_LINKS_PER_HOUR: i64 = 5;

/// Body of `POST /admin/website/sign-in-link`.
#[derive(Debug, Deserialize)]
pub struct SignInLinkRequest {
    /// The address the visitor typed
    pub email: String,
    /// The docs page to return to once signed in
    pub next: String,
    /// The visitor's docs locale, carried back to the callback
    pub lang: String,
}

/// Body of `POST /admin/website/sign-in-link/consume`.
#[derive(Debug, Deserialize)]
pub struct ConsumeSignInLinkRequest {
    /// The `<selector>.<verifier>` token from the mailed link
    pub token: String,
}

/// The account a redeemed link signs in.
#[derive(Debug, Serialize)]
pub struct SignedInAccount {
    /// The account's id
    pub user_id: Uuid,
    /// The account's normalized address
    pub email: String,
}

/// The one answer the send route gives every valid request.
fn ok_status() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Trim, lowercase and validate an address, the way registration does.
fn valid_email(raw: &str) -> AppResult<String> {
    let email = normalize_email(raw);
    if AuthService::is_valid_email(&email) && !email.contains(char::is_whitespace) {
        Ok(email)
    } else {
        Err(AppError::invalid_input("Invalid email format"))
    }
}

/// Whether `next` is a docs page on the website: `/docs` or `/fr/docs`,
/// followed only by lowercase slug segments.
///
/// The value is carried into the mailed link and the Worker redirects to it
/// after sign-in, so anything wider (a scheme, `//host`, `..`, a query) is an
/// open redirect in waiting and is refused rather than cleaned.
fn is_docs_path(next: &str) -> bool {
    let rest = next
        .strip_prefix("/fr/docs")
        .or_else(|| next.strip_prefix("/docs"));
    let Some(rest) = rest else {
        return false;
    };
    if rest.is_empty() {
        return true;
    }
    let Some(segments) = rest.strip_prefix('/') else {
        return false;
    };
    segments.split('/').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// `POST /admin/website/sign-in-link` — mail a docs sign-in link.
///
/// The link goes out only when the address belongs to an active account that
/// is within its send budget; every valid request gets the same answer.
///
/// # Errors
///
/// `PermissionDenied` without `ManageWebsite`; `InvalidInput` for a malformed
/// address, a `next` outside the docs, or an unknown locale. Nothing after
/// validation reaches the caller: lookup, mint and send failures are logged.
pub async fn handle_send_sign_in_link(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<SignInLinkRequest>,
) -> AppResult<Json<Value>> {
    admin_token.require_permission(&AdminPermission::ManageWebsite)?;
    let email = valid_email(&request.email)?;
    if !is_docs_path(&request.next) {
        return Err(AppError::invalid_input("next must be a docs page"));
    }
    if !matches!(request.lang.as_str(), "en" | "fr") {
        return Err(AppError::invalid_input("lang must be en or fr"));
    }

    send_sign_in_link(&context, &email, &request.next, &request.lang).await;
    Ok(ok_status())
}

/// Mint and mail a link when the address earns one. Best-effort throughout:
/// the caller answers the same whatever happens here.
async fn send_sign_in_link(context: &AdminApiContext, email: &str, next: &str, lang: &str) {
    match mint_sign_in_url(context, email, next, lang).await {
        Ok(Some((user, url))) => deliver_sign_in_link(context, &user, &url).await,
        Ok(None) => {}
        Err(e) => error!(error = %e, "Website sign-in link: not minted"),
    }
}

/// The account and its link, when the address belongs to an active account
/// that is within its send budget; `None` when it earns no link.
///
/// # Errors
///
/// Returns the repository's error when the lookup, the count or the store fails.
async fn mint_sign_in_url(
    context: &AdminApiContext,
    email: &str,
    next: &str,
    lang: &str,
) -> AppResult<Option<(User, String)>> {
    let Some(user) = context.repos.users.get_by_email(email).await? else {
        return Ok(None);
    };
    if user.user_status != UserStatus::Active {
        return Ok(None);
    }

    let tokens = &context.repos.website_sign_in_tokens;
    let sent = tokens
        .count_recent_tokens(user.id, Utc::now() - Duration::hours(1))
        .await?;
    if sent >= SIGN_IN_LINKS_PER_HOUR {
        warn!(user_id = %user.id, sent, "Website sign-in link: hourly send budget spent");
        return Ok(None);
    }

    let generated = generate_link_token();
    tokens
        .store_token(
            user.id,
            &generated.selector,
            &generated.verifier_hash,
            SIGN_IN_LINK_TTL_MINUTES,
        )
        .await?;

    let url = sign_in_url(&context.website_base_url, &generated.token, next, lang);
    Ok(Some((user, url)))
}

/// The website callback a mailed link lands on, every query value
/// percent-encoded.
fn sign_in_url(website_base_url: &str, token: &str, next: &str, lang: &str) -> String {
    format!(
        "{website_base_url}/docs/auth/callback?token={}&next={}&lang={}",
        urlencoding::encode(token),
        urlencoding::encode(next),
        urlencoding::encode(lang)
    )
}

/// Mail a minted link to its account.
///
/// The token is stored before the email service is looked at on purpose: its
/// row records that the address earned a link, and a link no mail carries
/// expires unredeemed.
async fn deliver_sign_in_link(context: &AdminApiContext, user: &User, url: &str) {
    let Some(email_service) = context.email_service.as_ref() else {
        warn!(user_id = %user.id, "Website sign-in link: email service not configured, nothing sent");
        return;
    };
    match email_service
        .send_website_sign_in_link(&user.email, url, SIGN_IN_LINK_TTL_MINUTES)
        .await
    {
        Ok(()) => info!(user_id = %user.id, "Website sign-in link sent"),
        Err(e) => error!(user_id = %user.id, error = %e, "Website sign-in link: send failed"),
    }
}

/// Collapse any redemption failure into the one uniform answer, logging it
/// first when it was the server's fault rather than the link's.
fn uniform_failure(e: &AppError) -> AppError {
    if e.is_server_fault() {
        error!(error = %e, "Website sign-in link: redemption failed");
    }
    invalid_sign_in_token()
}

/// `POST /admin/website/sign-in-link/consume` — redeem a link once.
///
/// The account must still exist and still be active when the link is redeemed:
/// a suspension between the send and the click closes the link.
///
/// # Errors
///
/// `PermissionDenied` without `ManageWebsite`; one uniform `NotFound` for every
/// other failure (malformed, unknown, expired, used, locked out, account gone
/// or no longer active), so the answer never says which.
pub async fn handle_consume_sign_in_link(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<ConsumeSignInLinkRequest>,
) -> AppResult<Json<SignedInAccount>> {
    admin_token.require_permission(&AdminPermission::ManageWebsite)?;
    let Some((selector, verifier_hash)) = split_link_token(request.token.trim()) else {
        return Err(invalid_sign_in_token());
    };

    let user_id = context
        .repos
        .website_sign_in_tokens
        .consume_token(&selector, &verifier_hash)
        .await
        .map_err(|e| uniform_failure(&e))?;

    let user = context
        .repos
        .users
        .get_global(user_id)
        .await
        .map_err(|e| uniform_failure(&e))?
        .filter(|user| user.user_status == UserStatus::Active)
        .ok_or_else(invalid_sign_in_token)?;

    info!(user_id = %user.id, "Website sign-in link redeemed");
    Ok(Json(SignedInAccount {
        user_id: user.id,
        email: normalize_email(&user.email),
    }))
}

#[cfg(test)]
mod tests {
    use super::{is_docs_path, sign_in_url};

    #[test]
    fn docs_pages_in_either_locale_are_return_targets() {
        assert!(is_docs_path("/docs"));
        assert!(is_docs_path("/fr/docs"));
        assert!(is_docs_path("/docs/coach-onboarding"));
        assert!(is_docs_path("/fr/docs/connect-your-data"));
    }

    #[test]
    fn anything_wider_than_a_docs_slug_is_refused() {
        for next in [
            "https://evil.com",
            "//evil.com",
            "/docs/../admin",
            "/docs?x=1",
            "/docsx",
            "/docs/Upper",
            "/docs//x",
            "/docs/",
            "/en/docs",
            "",
        ] {
            assert!(!is_docs_path(next), "{next:?} must not be a return target");
        }
    }

    #[test]
    fn the_link_lands_on_the_website_callback_with_every_value_encoded() {
        let url = sign_in_url(
            "https://dravr.ai",
            "sel.ver",
            "/fr/docs/connect-your-data",
            "fr",
        );
        assert_eq!(
            url,
            "https://dravr.ai/docs/auth/callback?token=sel.ver&next=%2Ffr%2Fdocs%2Fconnect-your-data&lang=fr"
        );
    }
}
