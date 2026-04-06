// ABOUTME: HTML template rendering for the Sciotte hosted-login flow
// ABOUTME: Uses compile-time embedded templates with HTML attribute escaping for XSS prevention
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Templates for the channel-initiated Sciotte hosted-login pages.
//!
//! Three pages share the Pierre brand styling used by `oauth_login.html` and
//! `messaging_link_login.html`:
//!
//! - Login page: embeds a signed link-token and a `target` (strava/garmin) and
//!   runs the full Sciotte login state machine in vanilla JS.
//! - Success page: shown after a successful connection.
//! - Error page: shown when the link-token is missing/invalid, or when the user
//!   clicks an explicit error link.

use crate::utils::html::escape_html_attribute;

const LOGIN_TEMPLATE: &str = include_str!("../../../templates/sciotte_link_login.html");
const SUCCESS_TEMPLATE: &str = include_str!("../../../templates/sciotte_link_success.html");
const ERROR_TEMPLATE: &str = include_str!("../../../templates/sciotte_link_error.html");

/// Render the Sciotte hosted-login page, embedding the signed link-token and
/// the target platform into the page's JS config.
#[must_use]
pub fn render_login_page(link_token: &str, target: &str, channel: &str) -> String {
    let target_label = match target {
        "garmin" => "Garmin",
        _ => "Strava",
    };
    let channel_label = humanize_channel(channel);

    LOGIN_TEMPLATE
        .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
        .replace("{{TARGET}}", &escape_html_attribute(target))
        .replace("{{TARGET_LABEL}}", &escape_html_attribute(target_label))
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CHANNEL_LABEL}}", &escape_html_attribute(&channel_label))
}

/// Render the success page shown after a successful Sciotte connection.
#[must_use]
pub fn render_success_page(channel: &str, target: &str) -> String {
    let target_label = match target {
        "garmin" => "Garmin",
        _ => "Strava",
    };
    let channel_label = humanize_channel(channel);

    SUCCESS_TEMPLATE
        .replace("{{TARGET_LABEL}}", &escape_html_attribute(target_label))
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CHANNEL_LABEL}}", &escape_html_attribute(&channel_label))
}

/// Render the error page for invalid/expired links or other hosted-login failures.
#[must_use]
pub fn render_error_page(message: &str) -> String {
    ERROR_TEMPLATE.replace("{{MESSAGE}}", &escape_html_attribute(message))
}

/// Convert a channel slug to a user-facing label ("slack" -> "Slack")
fn humanize_channel(slug: &str) -> String {
    match slug {
        "" => "your chat app".to_owned(),
        "whatsapp" => "WhatsApp".to_owned(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        }
    }
}
