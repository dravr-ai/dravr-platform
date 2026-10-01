// ABOUTME: HTML rendering for the channel-initiated hosted connect picker page
// ABOUTME: Embeds the connect link-token + provider catalogue; reuses the Sciotte success/error pages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Templates for the channel-initiated hosted **connect picker** page.
//!
//! The picker lets a messaging user choose a provider and connect via OAuth or
//! the Sciotte credential state machine — mirroring the web onboarding cards.
//! It shares the Boreal stylesheet every hosted page embeds and the
//! success/error pages with the Sciotte hosted-login flow
//! ([`crate::sciotte_hosted_templates`]).

use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_core::models::messaging::channel_label;
use urlencoding::encode;

use crate::sciotte_hosted_templates;

const CONNECT_TEMPLATE: &str = include_str!("../templates/connect_hosted.html");
const INTERVALS_ICU_TEMPLATE: &str = include_str!("../templates/connect_intervals_icu.html");

/// Render the hosted connect picker, embedding the signed connect link-token,
/// the originating channel, and the provider catalogue JSON the page renders
/// into selectable cards.
///
/// `providers_json` is a serialized JSON array of
/// `{ provider, display_name, description, connected, kind, target, … }` produced by
/// [`crate::connect_hosted::build_connect_providers`]. It is injected verbatim
/// into a `<script>` literal, so it MUST be valid, already-escaped JSON.
#[must_use]
pub fn render_connect_page(link_token: &str, channel: &str, providers_json: &str) -> String {
    // The notice starts hidden and empty: when a card whose
    // `consent_required` is set is picked, the page fills the block with that
    // card's own `notice` and shows it.
    sciotte_hosted_templates::fill_consent(&with_hosted_page_css(CONNECT_TEMPLATE), true, "")
        .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace(
            "{{CHANNEL_LABEL}}",
            &escape_html_attribute(channel_label(channel)),
        )
        // PROVIDERS_JSON is injected last so an escaped token/channel can never
        // close the script context before it.
        .replace("{{PROVIDERS_JSON}}", providers_json)
}

/// Render the hosted Intervals.icu API-key form for the connect link-token.
///
/// `athlete_id` refills the athlete id field after a rejected attempt; the API
/// key is never written back into the page. `error` is shown in the page's
/// alert banner, which stays hidden when it is empty.
#[must_use]
pub fn render_intervals_icu_form(
    link_token: &str,
    channel: &str,
    athlete_id: &str,
    error: &str,
) -> String {
    // The typed values go in last, so an athlete id or error that spells a
    // placeholder is never expanded.
    with_hosted_page_css(INTERVALS_ICU_TEMPLATE)
        .replace(
            "{{LINK_TOKEN_QUERY}}",
            &escape_html_attribute(&encode(link_token)),
        )
        .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
        .replace(
            "{{CHANNEL_LABEL}}",
            &escape_html_attribute(channel_label(channel)),
        )
        .replace(
            "{{ERROR_HIDDEN}}",
            if error.is_empty() { " hidden" } else { "" },
        )
        .replace("{{ERROR_MESSAGE}}", &escape_html_attribute(error))
        .replace("{{ATHLETE_ID}}", &escape_html_attribute(athlete_id))
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
