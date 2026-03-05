// ABOUTME: HTML template rendering for messaging channel link login, success, and error pages
// ABOUTME: Uses compile-time embedded templates with HTML attribute escaping for XSS prevention
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::response::Html;

use crate::utils::html::escape_html_attribute;

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
            r#"<div class="error-message">{}</div>"#,
            escape_html_attribute(msg)
        )
    });

    let html = LINK_LOGIN_TEMPLATE
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CODE}}", &escape_html_attribute(code))
        .replace("{{GREETING}}", &greeting)
        .replace("{{ERROR}}", &error_html);

    Html(html)
}

/// Render the success page after a channel has been linked
pub fn render_link_success_page(channel: &str) -> Html<String> {
    let html = LINK_SUCCESS_TEMPLATE.replace("{{CHANNEL}}", &escape_html_attribute(channel));
    Html(html)
}

/// Render the error page for expired or invalid link codes
pub fn render_link_error_page(message: &str) -> Html<String> {
    let html = LINK_ERROR_TEMPLATE.replace("{{MESSAGE}}", &escape_html_attribute(message));
    Html(html)
}
