// ABOUTME: Browser-facing approval page for the RFC 8628 device flow — the gcloud-style zero-token bootstrap
// ABOUTME: The operator signs in as a super-admin (email/password) in the browser to approve a pending CLI login
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Browser approval surface for `pierre-cli auth login`.
//!
//! Unlike the token-gated `POST /admin/device/approve` (a headless/CI fallback),
//! this is the primary, gcloud-style path: the operator opens
//! `GET /admin/device?user_code=XXXX` and approves by signing in **as a
//! super-admin** (email/password, verified directly against the API host). No
//! pre-existing token is needed — the browser sign-in *is* the authorization.
//!
//! Approval is **credential-gated** (re-auth per approval, sudo-style) rather
//! than ambient-session-gated. Because the request carries the password in its
//! body, a cross-site attacker cannot forge it, so `POST
//! /admin/device/approve-web` is CSRF-exempt (see `CSRF_EXEMPT_PATHS`) and needs
//! no session cookie, CSRF header, or JavaScript.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::header::RETRY_AFTER;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use chrono::Utc;
use serde::Deserialize;
use tracing::{error, info, warn};

use pierre_auth::password::verify_password;
use pierre_core::html::{escape_html_attribute, HOSTED_PAGE_CSS};
use pierre_core::models::User;
use pierre_middleware::PeerAddress;

use crate::context::AdminApiContext;

/// Query string of the approval page: the `user_code` the CLI is polling for.
#[derive(Deserialize)]
pub struct DevicePageQuery {
    /// The short device `user_code`, carried through to the approval form.
    pub user_code: Option<String>,
}

/// The sign-in-and-approve form submitted by the operator.
#[derive(Deserialize)]
pub struct DeviceApproveForm {
    /// Operator email (must resolve to a super-admin to approve).
    pub email: String,
    /// Operator password.
    pub password: String,
    /// The `user_code` being approved or denied.
    pub user_code: String,
    /// `approve` (default) or `deny`.
    pub action: Option<String>,
}

/// Verify email + password, returning the [`User`] if the credentials are valid.
///
/// Every failure reads as "not recognized" on the page, so the log is where a
/// lookup that failed is told apart from credentials that did not match. An
/// unknown address or a wrong password is counted in the sign-in windows of
/// `client` and `email` (carnet#804); a failed lookup is not the caller's
/// guess, so it is not.
async fn authenticate(
    context: &AdminApiContext,
    client: Option<IpAddr>,
    email: &str,
    password: &str,
) -> Option<User> {
    let user = match context.repos.users.get_by_email(email).await {
        Ok(user) => user,
        Err(e) => {
            error!(error = %e, "Device approval sign-in could not complete: user lookup failed");
            return None;
        }
    };
    // A fault inside the verifier is logged there; it answers false here.
    let verified = match user {
        Some(user) => verify_password(password.to_owned(), user.password_hash.clone())
            .await
            .unwrap_or(false)
            .then_some(user),
        None => None,
    };
    if verified.is_none() {
        context
            .sign_in_limiter
            .count_failed_sign_in(client, email)
            .await;
    }
    verified
}

/// Keep only the device `user_code` alphabet so it is safe in HTML and queries.
/// Values come from a fixed alphabet upstream; this is defence in depth.
fn sanitize_user_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(16)
        .collect()
}

/// `GET /admin/device?user_code=XXXX` — the sign-in-and-approve page.
pub async fn handle_device_page(Query(query): Query<DevicePageQuery>) -> Html<String> {
    let user_code = sanitize_user_code(query.user_code.as_deref().unwrap_or_default());
    Html(render_form(&user_code, None))
}

/// `POST /admin/device/approve-web` — sign in as a super-admin and approve/deny.
pub async fn handle_device_approve_web(
    State(context): State<Arc<AdminApiContext>>,
    peer: PeerAddress,
    headers: HeaderMap,
    Form(form): Form<DeviceApproveForm>,
) -> Response {
    let user_code = sanitize_user_code(&form.user_code);

    // The sign-in windows are read before the password is checked
    // (carnet#804): this page signs in a super-admin, the account most worth
    // guessing.
    let limiter = &context.sign_in_limiter;
    let client = peer.0.map(|peer| limiter.client_address(peer, &headers));
    match limiter.sign_in_wait(client, &form.email).await {
        Ok(None) => {}
        Ok(Some(retry_after)) => {
            warn!(
                retry_after,
                "Device approval sign-in refused: too many attempts"
            );
            let page = render_form(
                &user_code,
                Some("Too many sign-in attempts. Wait a minute, then try again."),
            );
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [(RETRY_AFTER, retry_after.to_string())],
                Html(page),
            )
                .into_response();
        }
        Err(e) => {
            error!(error = %e, "Device approval sign-in limiter could not read its windows");
            let page = render_form(
                &user_code,
                Some("Sign-in is temporarily unavailable. Try again in a minute."),
            );
            return (StatusCode::SERVICE_UNAVAILABLE, Html(page)).into_response();
        }
    }
    resolve_approval(&context, client, &user_code, &form)
        .await
        .into_response()
}

/// Sign the operator in and approve or deny `user_code`, once the sign-in
/// was admitted by its windows.
async fn resolve_approval(
    context: &AdminApiContext,
    client: Option<IpAddr>,
    user_code: &str,
    form: &DeviceApproveForm,
) -> Html<String> {
    let Some(user) = authenticate(context, client, &form.email, &form.password).await else {
        return Html(render_form(
            user_code,
            Some("That email or password was not recognized."),
        ));
    };
    if !user.role.is_super_admin() {
        return Html(render_form(
            user_code,
            Some(&format!(
                "{} is not a super-admin. Approving a CLI login requires a super-admin account.",
                user.email
            )),
        ));
    }

    let repo = context.repos.oauth2_server.as_ref();
    let Ok(Some(record)) = repo.get_device_authorization_by_user_code(user_code).await else {
        return Html(render_message(
            "Login request not found",
            "That code is unknown or has already been used. Start a new login from the CLI.",
        ));
    };
    if record.status != "pending" {
        return Html(render_message(
            "Already resolved",
            &format!("This login request is already {}.", record.status),
        ));
    }
    if record.expires_at <= Utc::now().timestamp() {
        return Html(render_message(
            "Expired",
            "This code has expired. Start a new login from the CLI.",
        ));
    }

    let deny = form.action.as_deref() == Some("deny");
    let outcome = if deny {
        repo.deny_device_authorization(user_code).await
    } else {
        repo.approve_device_authorization(user_code, &user.id.to_string())
            .await
    };

    match outcome {
        Ok(true) => {
            info!(
                user_code = %user_code,
                approver_id = %user.id,
                action = if deny { "deny" } else { "approve" },
                "Device login resolved via browser approval"
            );
            if deny {
                Html(render_message(
                    "Login denied",
                    "The CLI login request was denied. You can close this tab.",
                ))
            } else {
                Html(render_message(
                    "Login approved",
                    "The CLI can now finish signing in. You can close this tab.",
                ))
            }
        }
        _ => Html(render_message(
            "Could not complete",
            "The login request could not be updated. It may have expired — start a new one.",
        )),
    }
}

/// Wrap a page body in the hosted-page shell: the shared Boreal sheet, both
/// schemes, and the lockup at the top of the card.
fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<meta name=\"color-scheme\" content=\"light dark\">\
<title>{title}</title><style>{HOSTED_PAGE_CSS}</style></head><body><main class=\"card\">\
<div class=\"lockup\" role=\"img\" aria-label=\"Dravr\"></div>{body}</main></body></html>"
    )
}

fn render_form(user_code: &str, error: Option<&str>) -> String {
    let code = escape_html_attribute(user_code);
    let err = error.map_or_else(String::new, |e| {
        format!(
            "<p class=\"error-text\" role=\"alert\">{}</p>",
            escape_html_attribute(e)
        )
    });
    let body = format!(
        "<h1>Approve CLI sign-in</h1>\
<p class=\"lead\">Confirm the code shown by <code>pierre-cli auth login</code>, then sign in as a super-admin to approve.</p>\
<div class=\"user-code\">{code}</div>\
<form method=\"post\" action=\"/admin/device/approve-web\">\
<input type=\"hidden\" name=\"user_code\" value=\"{code}\">\
<div class=\"field\"><label for=\"email\">Email</label>\
<input id=\"email\" name=\"email\" type=\"email\" autocomplete=\"username\" required></div>\
<div class=\"field\"><label for=\"password\">Password</label>\
<input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required></div>\
<div class=\"actions\">\
<button class=\"btn btn-primary btn-block\" name=\"action\" value=\"approve\" type=\"submit\">Sign in &amp; approve</button>\
<button class=\"btn btn-secondary btn-block\" name=\"action\" value=\"deny\" type=\"submit\">Deny</button>\
</div></form>{err}"
    );
    page("Approve CLI sign-in", &body)
}

fn render_message(title: &str, message: &str) -> String {
    let body = format!(
        "<h1>{}</h1><p>{}</p>",
        escape_html_attribute(title),
        escape_html_attribute(message)
    );
    page(title, &body)
}
