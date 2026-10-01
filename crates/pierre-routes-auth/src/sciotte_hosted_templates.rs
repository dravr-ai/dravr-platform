// ABOUTME: HTML template rendering for the Sciotte hosted-login flow
// ABOUTME: Uses compile-time embedded templates with HTML attribute escaping for XSS prevention
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Templates for the channel-initiated Sciotte hosted-login pages.
//!
//! Three pages share the Boreal stylesheet every hosted page embeds
//! ([`pierre_core::html::with_hosted_page_css`]):
//!
//! - Login page: embeds a signed link-token and a `target` (strava / garmin /
//!   trainingpeaks) and
//!   runs the full Sciotte login state machine in vanilla JS.
//! - Success page: shown after a successful connection.
//! - Error page: shown when the link-token is missing/invalid, or when the user
//!   clicks an explicit error link.

use pierre_contremaitre::hosted_strings::{
    KEY_HOSTED_COMMON_ACCOUNT_TITLE, KEY_HOSTED_COMMON_CONNECTING_TO,
    KEY_HOSTED_COMMON_CREDENTIALS_NOTE, KEY_HOSTED_COMMON_EMAIL_LABEL,
    KEY_HOSTED_COMMON_LINKED_FROM, KEY_HOSTED_COMMON_LOADING, KEY_HOSTED_COMMON_SIGN_IN_EXPIRED,
    KEY_HOSTED_COMMON_SIGN_IN_REJECTED, KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE,
    KEY_HOSTED_COMMON_SUBTITLE_NAMED, KEY_HOSTED_COMMON_USERNAME_LABEL,
    KEY_HOSTED_COMMON_VERIFYING_CODE, KEY_HOSTED_ERROR_INVALID_LINK,
    KEY_HOSTED_LOGIN_DATA_AVAILABLE, KEY_HOSTED_SUCCESS_DATA_AVAILABLE,
    KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC, KEY_HOSTED_SUCCESS_HEADING,
    KEY_HOSTED_SUCCESS_HEADING_GENERIC, KEY_HOSTED_SUCCESS_RETURN_TO_CHAT,
};
use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use pierre_providers::sciotte_provider::SciotteTarget;
use serde::Serialize;
use serde_json::json;
use tracing::error;

use crate::hosted_page::{script_json, PageStrings};
use crate::sciotte::LOGIN_FLOW_EXPIRED_REASON;

const LOGIN_TEMPLATE: &str = include_str!("../templates/sciotte_link_login.html");
const SUCCESS_TEMPLATE: &str = include_str!("../templates/sciotte_link_success.html");
const ERROR_TEMPLATE: &str = include_str!("../templates/sciotte_link_error.html");

/// The catalogue entry (under `providers`) holding each provider's notice, by
/// the hosted-login target (TrainingPeaks, COROS) or the OAuth provider id
/// (WHOOP) the hosted pages name it by. A provider with no row asks for no
/// notice.
const NOTICE_KEYS: [(&str, &str); 3] = [
    ("trainingpeaks", "trainingpeaksNotice"),
    ("coros", "corosNotice"),
    ("whoop", "whoopNotice"),
];

/// A provider's exposure notice as the web and mobile modals show it: a
/// title, the body, and the acceptance its required checkbox states.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExposureNotice {
    /// The notice's heading
    pub title: String,
    /// The risk it states
    pub body: String,
    /// The acceptance its required checkbox states
    pub consent: String,
}

/// The notice a hosted-login target or an OAuth provider shows, in the page's
/// locale, or `None` when it asks for none.
///
/// Read from the catalogue entry the web and mobile modals show: the one
/// text, never a copy of it. The catalogue is compiled in and the hosted-page
/// route tests pin its entries, so a miss cannot reach a build that passed
/// them; if one does, the operator is paged and the page still serves — the
/// login handler refuses a login without the acceptance either way.
#[must_use]
pub fn exposure_notice(strings: &PageStrings<'_>, target: &str) -> Option<ExposureNotice> {
    let (_, entry) = NOTICE_KEYS.iter().find(|(t, _)| *t == target)?;
    let notice = ExposureNotice {
        title: strings.get(&format!("providers.{entry}.title")),
        body: strings.get(&format!("providers.{entry}.body")),
        consent: strings.get(&format!("providers.{entry}.consent")),
    };
    if notice.title.is_empty() || notice.body.is_empty() || notice.consent.is_empty() {
        error!(
            target = target,
            locale = strings.locale(),
            "the catalogue carries no notice for this target; the hosted pages show none"
        );
    }
    Some(notice)
}

/// Fill a template's consent placeholders with `notice`. `hidden` decides
/// whether the block starts visible. A target with no notice leaves the block
/// empty; it is shown only when a notice is required.
pub fn fill_consent(template: &str, hidden: bool, notice: Option<&ExposureNotice>) -> String {
    let empty = ExposureNotice::default();
    let notice = notice.unwrap_or(&empty);
    template
        .replace("{{CONSENT_HIDDEN}}", if hidden { " hidden" } else { "" })
        .replace("{{CONSENT_TITLE}}", &escape_html_attribute(&notice.title))
        .replace("{{CONSENT_NOTICE}}", &escape_html_attribute(&notice.body))
        .replace("{{CONSENT_LABEL}}", &escape_html_attribute(&notice.consent))
}

/// Render the Sciotte hosted-login page in the athlete's locale, embedding
/// the signed link-token and the target platform into the page's JS config.
///
/// `consent_required` shows the provider's exposure notice with a required
/// checkbox above the credentials, and makes the page send the acceptance.
#[must_use]
pub fn render_login_page(
    strings: &PageStrings<'_>,
    link_token: &str,
    target: &str,
    channel: &str,
    consent_required: bool,
) -> String {
    // The target is clamped to a hosted-login target before this renders, so
    // the registry always names it; the Strava label is the page's historical
    // default target.
    let target_label = backend_resolver::brand_name(&global_registry(), target).unwrap_or("Strava");
    let channel_label = strings.channel(channel);

    // The identifier the provider's own login asks for — a username for
    // TrainingPeaks, an email otherwise.
    let (id_label, id_type, id_autocomplete) =
        if SciotteTarget::from_target_param(target).signs_in_with_username() {
            (KEY_HOSTED_COMMON_USERNAME_LABEL, "text", "username")
        } else {
            (KEY_HOSTED_COMMON_EMAIL_LABEL, "email", "email")
        };

    // What the page's script writes: status lines, and every failure of the
    // sign-in — the script prints none of the server's or the provider's own
    // text.
    let connecting_to = strings.render(KEY_HOSTED_COMMON_CONNECTING_TO, &[target_label]);
    let script_strings = json!({
        "connectingTo": connecting_to,
        "loading": strings.get(KEY_HOSTED_COMMON_LOADING),
        "verifyingCode": strings.get(KEY_HOSTED_COMMON_VERIFYING_CODE),
        "signInRejected": strings.render(KEY_HOSTED_COMMON_SIGN_IN_REJECTED, &[target_label]),
        "signInUnavailable": strings.render(KEY_HOSTED_COMMON_SIGN_IN_UNAVAILABLE, &[target_label]),
        "signInExpired": strings.get(KEY_HOSTED_COMMON_SIGN_IN_EXPIRED),
        "linkExpired": strings.get(KEY_HOSTED_ERROR_INVALID_LINK),
    });

    let notice = exposure_notice(strings, target);
    fill_consent(
        &strings.fill(&with_hosted_page_css(LOGIN_TEMPLATE)),
        !consent_required,
        notice.as_ref(),
    )
    .replace(
        "{{CONSENT_REQUIRED}}",
        if consent_required { "true" } else { "false" },
    )
    .replace("{{ID_LABEL}}", &strings.html(id_label, &[]))
    .replace("{{ID_TYPE}}", id_type)
    .replace("{{ID_AUTOCOMPLETE}}", id_autocomplete)
    .replace(
        "{{SUBTITLE}}",
        &strings.html(KEY_HOSTED_COMMON_SUBTITLE_NAMED, &[target_label]),
    )
    .replace(
        "{{LINKED_FROM}}",
        &strings.html(KEY_HOSTED_COMMON_LINKED_FROM, &[&channel_label]),
    )
    .replace(
        "{{ACCOUNT_TITLE}}",
        &strings.html(KEY_HOSTED_COMMON_ACCOUNT_TITLE, &[target_label]),
    )
    .replace(
        "{{CREDENTIALS_NOTE}}",
        &strings.html(KEY_HOSTED_COMMON_CREDENTIALS_NOTE, &[target_label]),
    )
    .replace("{{CONNECTING_TO}}", &escape_html_attribute(&connecting_to))
    .replace(
        "{{DATA_AVAILABLE}}",
        &strings.html(
            KEY_HOSTED_LOGIN_DATA_AVAILABLE,
            &[target_label, &channel_label],
        ),
    )
    .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
    .replace("{{TARGET}}", &escape_html_attribute(target))
    .replace("{{CHANNEL}}", &escape_html_attribute(channel))
    .replace("{{FLOW_EXPIRED_REASON}}", LOGIN_FLOW_EXPIRED_REASON)
    // The script's strings go in last, so an escaped token or channel can
    // never close the script context before them.
    .replace("{{STRINGS_JSON}}", &script_json(&script_strings))
}

/// Render the success page shown after a successful connection, in the
/// athlete's locale. Reused by the hosted connect picker, so the label map
/// covers OAuth providers too; a target the registry does not name gets the
/// page's unnamed wording.
#[must_use]
pub fn render_success_page(strings: &PageStrings<'_>, channel: &str, target: &str) -> String {
    let (heading, data_available) = backend_resolver::brand_name(&global_registry(), target)
        .map_or_else(
            || {
                (
                    strings.html(KEY_HOSTED_SUCCESS_HEADING_GENERIC, &[]),
                    strings.html(KEY_HOSTED_SUCCESS_DATA_AVAILABLE_GENERIC, &[]),
                )
            },
            |label| {
                (
                    strings.html(KEY_HOSTED_SUCCESS_HEADING, &[label]),
                    strings.html(KEY_HOSTED_SUCCESS_DATA_AVAILABLE, &[label]),
                )
            },
        );

    strings
        .fill(&with_hosted_page_css(SUCCESS_TEMPLATE))
        .replace("{{HEADING}}", &heading)
        .replace("{{DATA_AVAILABLE}}", &data_available)
        .replace(
            "{{RETURN_TO_CHAT}}",
            &strings.html(
                KEY_HOSTED_SUCCESS_RETURN_TO_CHAT,
                &[&strings.channel(channel)],
            ),
        )
}

/// Render the error page for invalid/expired links or other hosted-login
/// failures: `message` under the page's own heading, both in its locale.
#[must_use]
pub fn render_error_page(strings: &PageStrings<'_>, message: &str) -> String {
    strings
        .fill(&with_hosted_page_css(ERROR_TEMPLATE))
        .replace("{{MESSAGE}}", &escape_html_attribute(message))
}
