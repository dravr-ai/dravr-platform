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

use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_COMMON_ACCOUNT_TITLE, KEY_HOSTED_COMMON_CONNECTING_TO,
    KEY_HOSTED_COMMON_CREDENTIALS_NOTE, KEY_HOSTED_COMMON_EMAIL_LABEL,
    KEY_HOSTED_COMMON_LINKED_FROM, KEY_HOSTED_COMMON_LOADING, KEY_HOSTED_COMMON_OTP_LABEL_NAMED,
    KEY_HOSTED_COMMON_SIGN_IN_EXPIRED, KEY_HOSTED_COMMON_SIGN_IN_REJECTED,
    KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE, KEY_HOSTED_COMMON_USERNAME_LABEL,
    KEY_HOSTED_COMMON_VERIFYING_CODE, KEY_HOSTED_ERROR_INVALID_LINK,
    KEY_HOSTED_PICKER_DATA_AVAILABLE, KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE,
    KEY_HOSTED_PICKER_STRAVA_FALLBACK, KEY_HOSTED_PICKER_TAG_API_KEY,
    KEY_HOSTED_PICKER_TAG_AUTHORIZE, KEY_HOSTED_PICKER_TAG_CONNECTED,
    KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD, KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD,
    KEY_SCIOTTE_CODE_REJECTED,
};
use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use serde_json::{json, Value};
use urlencoding::encode;

use crate::hosted_page::{script_json, PageStrings};
use crate::sciotte::{CODE_REJECTED_REASON, LOGIN_FLOW_EXPIRED_REASON};
use crate::sciotte_hosted_templates;

const CONNECT_TEMPLATE: &str = include_str!("../templates/connect_hosted.html");
const INTERVALS_ICU_TEMPLATE: &str = include_str!("../templates/connect_intervals_icu.html");

/// The brand the Intervals.icu form names its account by.
const INTERVALS_ICU_LABEL: &str = "Intervals.icu";

/// Render the hosted connect picker in the athlete's locale, embedding the
/// signed connect link-token, the originating channel, and the provider cards
/// the page renders.
///
/// `providers` is the array of
/// `{ provider, display_name, description, connected, kind, target, … }`
/// [`crate::connect_hosted::build_connect_providers`] produced. It and the
/// strings the page's script sets go into the `<script>` as JSON, so neither
/// is ever markup.
#[must_use]
pub fn render_connect_page(
    strings: &PageStrings<'_>,
    link_token: &str,
    channel: &str,
    providers: &Value,
) -> String {
    let channel_label = strings.channel(channel);
    // What the page's script writes: the card tags, the labels and notes that
    // name the picked provider, and every failure of a credential sign-in —
    // the script prints none of the server's or the provider's own text. A
    // `{0}` is the slot the script fills with the provider.
    let script_strings = json!({
        "email": strings.get(KEY_HOSTED_COMMON_EMAIL_LABEL),
        "username": strings.get(KEY_HOSTED_COMMON_USERNAME_LABEL),
        "tagConnected": strings.get(KEY_HOSTED_PICKER_TAG_CONNECTED),
        "tagAuthorize": strings.get(KEY_HOSTED_PICKER_TAG_AUTHORIZE),
        "tagApiKey": strings.get(KEY_HOSTED_PICKER_TAG_API_KEY),
        "tagUsernamePassword": strings.get(KEY_HOSTED_PICKER_TAG_USERNAME_PASSWORD),
        "tagEmailPassword": strings.get(KEY_HOSTED_PICKER_TAG_EMAIL_PASSWORD),
        "accountTitle": strings.get(KEY_HOSTED_COMMON_ACCOUNT_TITLE),
        "credentialsNote": strings.get(KEY_HOSTED_COMMON_CREDENTIALS_NOTE),
        "connectingTo": strings.get(KEY_HOSTED_COMMON_CONNECTING_TO),
        "loading": strings.get(KEY_HOSTED_COMMON_LOADING),
        "verifyingCode": strings.get(KEY_HOSTED_COMMON_VERIFYING_CODE),
        "otpLabel": strings.get(KEY_HOSTED_COMMON_OTP_LABEL_NAMED),
        "codeRejected": strings.get(KEY_SCIOTTE_CODE_REJECTED),
        "signInRejected": strings.get(KEY_HOSTED_COMMON_SIGN_IN_REJECTED),
        "signInUnavailable": strings.get(KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE),
        "signInExpired": strings.get(KEY_HOSTED_COMMON_SIGN_IN_EXPIRED),
        "linkExpired": strings.get(KEY_HOSTED_ERROR_INVALID_LINK),
        "stravaFallback": strings.get(KEY_HOSTED_PICKER_STRAVA_FALLBACK),
        "signInIncomplete": strings.get(KEY_HOSTED_PICKER_SIGN_IN_INCOMPLETE),
    });

    // The notice starts hidden and empty: when a card whose
    // `consent_required` is set is picked, the page fills the block with that
    // card's own `notice` and shows it.
    sciotte_hosted_templates::fill_consent(
        &strings.fill(&with_hosted_page_css(CONNECT_TEMPLATE)),
        true,
        None,
    )
    .replace(
        "{{LINKED_FROM}}",
        &strings.html(KEY_HOSTED_COMMON_LINKED_FROM, &[&channel_label]),
    )
    .replace(
        "{{DATA_AVAILABLE}}",
        &strings.html(KEY_HOSTED_PICKER_DATA_AVAILABLE, &[&channel_label]),
    )
    .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
    .replace("{{CHANNEL}}", &escape_html_attribute(channel))
    .replace("{{FLOW_EXPIRED_REASON}}", LOGIN_FLOW_EXPIRED_REASON)
    .replace("{{CODE_REJECTED_REASON}}", CODE_REJECTED_REASON)
    // The JSON goes in last so an escaped token/channel can never close the
    // script context before it.
    .replace("{{STRINGS_JSON}}", &script_json(&script_strings))
    .replace("{{PROVIDERS_JSON}}", &script_json(providers))
}

/// Render the hosted Intervals.icu API-key form for the connect link-token,
/// in the athlete's locale.
///
/// `athlete_id` refills the athlete id field after a rejected attempt; the API
/// key is never written back into the page. `error` is the already-localized
/// reason shown in the page's alert banner, which stays hidden when it is
/// empty.
#[must_use]
pub fn render_intervals_icu_form(
    strings: &PageStrings<'_>,
    link_token: &str,
    channel: &str,
    athlete_id: &str,
    error: &str,
) -> String {
    // The typed values go in last, so an athlete id or error that spells a
    // placeholder is never expanded.
    strings
        .fill(&with_hosted_page_css(INTERVALS_ICU_TEMPLATE))
        .replace(
            "{{LINKED_FROM}}",
            &strings.html(KEY_HOSTED_COMMON_LINKED_FROM, &[&strings.channel(channel)]),
        )
        .replace(
            "{{ACCOUNT_TITLE}}",
            &strings.html(KEY_HOSTED_COMMON_ACCOUNT_TITLE, &[INTERVALS_ICU_LABEL]),
        )
        .replace(
            "{{LINK_TOKEN_QUERY}}",
            &escape_html_attribute(&encode(link_token)),
        )
        .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
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
pub fn render_connect_success_page(
    strings: &PageStrings<'_>,
    channel: &str,
    target: &str,
) -> String {
    sciotte_hosted_templates::render_success_page(strings, channel, target)
}

/// Render the connect error page (reuses the Sciotte error template) with the
/// catalogue's `message_key` as its message.
#[must_use]
pub fn render_connect_error_page(strings: &PageStrings<'_>, message_key: &str) -> String {
    sciotte_hosted_templates::render_error_page(strings, &strings.get(message_key))
}
