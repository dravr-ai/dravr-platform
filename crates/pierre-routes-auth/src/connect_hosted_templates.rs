// ABOUTME: HTML rendering for the channel-initiated hosted connect picker page
// ABOUTME: Embeds the connect link-token + provider catalogue; reuses the Sciotte success/error pages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Templates for the channel-initiated hosted **connect picker** page.
//!
//! The picker lets a messaging user choose a provider and connect via OAuth or
//! the Sciotte credential state machine — mirroring the web onboarding cards.
//! It shares the Pierre brand styling and the success/error pages with the
//! Sciotte hosted-login flow ([`crate::sciotte_hosted_templates`]).

use pierre_core::html::escape_html_attribute;

use crate::sciotte_hosted_templates;

const CONNECT_TEMPLATE: &str = include_str!("../templates/connect_hosted.html");

/// Render the hosted connect picker, embedding the signed connect link-token,
/// the originating channel, and the provider catalogue JSON the page renders
/// into selectable cards.
///
/// `providers_json` is a serialized JSON array of
/// `{ provider, display_name, connected, kind, target }` produced by
/// [`crate::connect_hosted::build_connect_providers`]. It is injected verbatim
/// into a `<script>` literal, so it MUST be valid, already-escaped JSON.
#[must_use]
pub fn render_connect_page(link_token: &str, channel: &str, providers_json: &str) -> String {
    let channel_label = humanize_channel(channel);

    CONNECT_TEMPLATE
        .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CHANNEL_LABEL}}", &escape_html_attribute(&channel_label))
        // PROVIDERS_JSON is injected last so an escaped token/channel can never
        // close the script context before it.
        .replace("{{PROVIDERS_JSON}}", providers_json)
}

/// Render the connect success page (reuses the Sciotte success template, which
/// is provider-agnostic beyond its label).
#[must_use]
pub fn render_connect_success_page(channel: &str, target: &str) -> String {
    sciotte_hosted_templates::render_success_page(channel, target)
}

/// Render the connect error page (reuses the Sciotte error template).
#[must_use]
pub fn render_connect_error_page(message: &str) -> String {
    sciotte_hosted_templates::render_error_page(message)
}

/// Convert a channel slug to a user-facing label ("slack" -> "Slack").
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
