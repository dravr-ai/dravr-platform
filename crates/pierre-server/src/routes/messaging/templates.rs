// ABOUTME: HTML template rendering for messaging channel link login, success, and error pages
// ABOUTME: Uses compile-time embedded templates with HTML attribute escaping for XSS prevention
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::response::Html;

use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_core::models::UserStatus;

/// Login page template embedded at compile-time
const LINK_LOGIN_TEMPLATE: &str = include_str!("../../../templates/messaging_link_login.html");

/// Success page template embedded at compile-time
const LINK_SUCCESS_TEMPLATE: &str = include_str!("../../../templates/messaging_link_success.html");

/// Error page template embedded at compile-time
const LINK_ERROR_TEMPLATE: &str = include_str!("../../../templates/messaging_link_error.html");

/// Render the channel linking login/register page
pub fn render_link_login_page(
    channel: &str,
    sender_name: Option<&str>,
    code: &str,
    error: Option<&str>,
) -> Html<String> {
    let greeting = sender_name.map_or_else(String::new, |name| {
        format!(
            r#"<div class="greeting">Hi <strong>{}</strong>, log in or create an account to connect.</div>"#,
            escape_html_attribute(name)
        )
    });

    let error_html = error.map_or_else(String::new, |msg| {
        format!(
            r#"<div class="alert alert-error" role="alert">{}</div>"#,
            escape_html_attribute(msg)
        )
    });

    let html = with_hosted_page_css(LINK_LOGIN_TEMPLATE)
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CODE}}", &escape_html_attribute(code))
        .replace("{{GREETING}}", &greeting)
        .replace("{{ERROR}}", &error_html);

    Html(html)
}

/// Render the success page after a channel has been linked.
///
/// The link is made whatever the account's status, as the in-chat flow makes
/// it; what the athlete can do next is not the same. An active account can
/// talk to the agent now, a pending one waits for an administrator, and a
/// suspended one is refused on every message until support restores it.
pub fn render_link_success_page(channel: &str, status: UserStatus) -> Html<String> {
    let next_step = match status {
        UserStatus::Active => {
            "Go back to {{CHANNEL}} and send a message to get started."
        }
        UserStatus::Pending => {
            "Your Dravr account is waiting for an administrator's approval. The agent answers on {{CHANNEL}} once it is approved."
        }
        UserStatus::Suspended => {
            "Your Dravr account is suspended, so the agent does not answer on {{CHANNEL}}. Contact support to restore it."
        }
    };
    let html = with_hosted_page_css(LINK_SUCCESS_TEMPLATE)
        .replace("{{NEXT_STEP}}", next_step)
        .replace("{{CHANNEL}}", &escape_html_attribute(channel));
    Html(html)
}

/// Render the error page for expired or invalid link codes
pub fn render_link_error_page(message: &str) -> Html<String> {
    let html = with_hosted_page_css(LINK_ERROR_TEMPLATE)
        .replace("{{MESSAGE}}", &escape_html_attribute(message));
    Html(html)
}
