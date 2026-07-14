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

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Html;
use axum::Form;
use chrono::Utc;
use serde::Deserialize;
use tokio::task::spawn_blocking;
use tracing::info;

use pierre_core::html::escape_html_attribute;
use pierre_core::models::User;

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
async fn authenticate(context: &AdminApiContext, email: &str, password: &str) -> Option<User> {
    let user = context.repos.users.get_by_email(email).await.ok()??;
    let hash = user.password_hash.clone();
    let candidate = password.to_owned();
    let verified = spawn_blocking(move || bcrypt::verify(&candidate, &hash))
        .await
        .ok()?
        .unwrap_or(false);
    verified.then_some(user)
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
    Form(form): Form<DeviceApproveForm>,
) -> Html<String> {
    let user_code = sanitize_user_code(&form.user_code);

    let Some(user) = authenticate(&context, &form.email, &form.password).await else {
        return Html(render_form(
            &user_code,
            Some("That email or password was not recognized."),
        ));
    };
    if !user.role.is_super_admin() {
        return Html(render_form(
            &user_code,
            Some(&format!(
                "{} is not a super-admin. Approving a CLI login requires a super-admin account.",
                user.email
            )),
        ));
    }

    let repo = context.repos.oauth2_server.as_ref();
    let Ok(Some(record)) = repo.get_device_authorization_by_user_code(&user_code).await else {
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
        repo.deny_device_authorization(&user_code).await
    } else {
        repo.approve_device_authorization(&user_code, &user.email)
            .await
    };

    match outcome {
        Ok(true) => {
            info!(
                user_code = %user_code,
                approver = %user.email,
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

const PAGE_STYLE: &str = "body{font-family:system-ui,-apple-system,sans-serif;background:#0f1115;color:#e6e6e6;\
display:flex;min-height:100vh;align-items:center;justify-content:center;margin:0}\
.card{background:#181b21;border:1px solid #262a33;border-radius:12px;padding:32px;max-width:400px;width:90%}\
h1{font-size:20px;margin:0 0 4px}p{color:#9aa4b2;font-size:14px;line-height:1.5}\
label{display:block;font-size:13px;margin:16px 0 6px;color:#c7cfdb}\
input{width:100%;box-sizing:border-box;padding:10px 12px;border-radius:8px;border:1px solid #2c313b;\
background:#0f1115;color:#e6e6e6;font-size:14px}\
button{margin-top:20px;width:100%;padding:11px;border:0;border-radius:8px;font-size:15px;font-weight:600;cursor:pointer}\
.primary{background:#3b82f6;color:#fff}.danger{background:transparent;color:#9aa4b2;border:1px solid #2c313b;margin-top:10px}\
.code{font-family:ui-monospace,monospace;font-size:22px;letter-spacing:2px;color:#fff;\
background:#0f1115;border:1px solid #2c313b;border-radius:8px;padding:12px;text-align:center;margin:8px 0 4px}\
.err{color:#f87171;font-size:13px;margin-top:12px}";

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
<title>{title}</title><style>{PAGE_STYLE}</style></head><body><div class=\"card\">{body}</div></body></html>"
    )
}

fn render_form(user_code: &str, error: Option<&str>) -> String {
    let code = escape_html_attribute(user_code);
    let err = error.map_or_else(String::new, |e| {
        format!("<p class=\"err\">{}</p>", escape_html_attribute(e))
    });
    let body = format!(
        "<h1>Approve CLI sign-in</h1>\
<p>Confirm the code shown by <code>pierre-cli auth login</code>, then sign in as a super-admin to approve.</p>\
<div class=\"code\">{code}</div>\
<form method=\"post\" action=\"/admin/device/approve-web\">\
<input type=\"hidden\" name=\"user_code\" value=\"{code}\">\
<label>Email</label><input name=\"email\" type=\"email\" autocomplete=\"username\" required>\
<label>Password</label><input name=\"password\" type=\"password\" autocomplete=\"current-password\" required>\
<button class=\"primary\" name=\"action\" value=\"approve\" type=\"submit\">Sign in &amp; approve</button>\
<button class=\"danger\" name=\"action\" value=\"deny\" type=\"submit\">Deny</button></form>{err}"
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
