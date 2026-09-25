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

use std::sync::LazyLock;

use pierre_core::html::{escape_html_attribute, with_hosted_page_css};
use pierre_providers::backend_resolver;
use pierre_providers::registry::global_registry;
use pierre_providers::sciotte_provider::SciotteTarget;
use serde::Serialize;
use serde_json::Value;
use tracing::error;

const LOGIN_TEMPLATE: &str = include_str!("../templates/sciotte_link_login.html");
const SUCCESS_TEMPLATE: &str = include_str!("../templates/sciotte_link_success.html");
const ERROR_TEMPLATE: &str = include_str!("../templates/sciotte_link_error.html");

/// The English catalogue the web and mobile clients read. The hosted pages
/// have no locale mechanism, so they show its English entry of each exposure
/// notice: the one text, never a copy of it.
const EN_CATALOGUE: &str = include_str!("../../../packages/i18n/src/locales/en/translation.json");

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

/// Every target's exposure notice, read once from [`EN_CATALOGUE`].
///
/// The catalogue is compiled in and the hosted-page route tests pin its
/// entries, so a miss cannot reach a build that passed them; if one does, the
/// operator is paged and the page still serves — the login handler refuses a
/// login without the acceptance either way.
static NOTICES: LazyLock<Vec<(&'static str, ExposureNotice)>> = LazyLock::new(|| {
    let catalogue = serde_json::from_str::<Value>(EN_CATALOGUE).unwrap_or(Value::Null);
    NOTICE_KEYS
        .iter()
        .map(|(target, key)| {
            let entry = &catalogue["providers"][key];
            let read = || {
                Some(ExposureNotice {
                    title: entry["title"].as_str()?.to_owned(),
                    body: entry["body"].as_str()?.to_owned(),
                    consent: entry["consent"].as_str()?.to_owned(),
                })
            };
            let notice = read().unwrap_or_else(|| {
                error!(target = target, "the en catalogue carries no notice for this target; the hosted pages show none");
                ExposureNotice::default()
            });
            (*target, notice)
        })
        .collect()
});

/// The notice a hosted-login target or an OAuth provider shows, or `None`
/// when it asks for none.
#[must_use]
pub fn exposure_notice(target: &str) -> Option<&'static ExposureNotice> {
    NOTICES
        .iter()
        .find(|(t, _)| *t == target)
        .map(|(_, notice)| notice)
}

/// Fill a template's consent placeholders with `target`'s notice. `hidden`
/// decides whether the block starts visible. A target with no notice leaves
/// the block empty; it is shown only when a notice is required.
pub fn fill_consent(template: &str, hidden: bool, target: &str) -> String {
    let notice = exposure_notice(target).cloned().unwrap_or_default();
    template
        .replace("{{CONSENT_HIDDEN}}", if hidden { " hidden" } else { "" })
        .replace("{{CONSENT_TITLE}}", &escape_html_attribute(&notice.title))
        .replace("{{CONSENT_NOTICE}}", &escape_html_attribute(&notice.body))
        .replace("{{CONSENT_LABEL}}", &escape_html_attribute(&notice.consent))
}

/// Render the Sciotte hosted-login page, embedding the signed link-token and
/// the target platform into the page's JS config.
///
/// `consent_required` shows the provider's exposure notice with a required
/// checkbox above the credentials, and makes the page send the acceptance.
#[must_use]
pub fn render_login_page(
    link_token: &str,
    target: &str,
    channel: &str,
    consent_required: bool,
) -> String {
    // The target is clamped to a hosted-login target before this renders, so
    // the registry always names it; the Strava label is the page's historical
    // default target.
    let target_label = backend_resolver::brand_name(&global_registry(), target).unwrap_or("Strava");
    let channel_label = humanize_channel(channel);

    // The identifier the provider's own login asks for — a username for
    // TrainingPeaks, an email otherwise.
    let (id_label, id_type, id_autocomplete) =
        if SciotteTarget::from_target_param(target).signs_in_with_username() {
            ("Username", "text", "username")
        } else {
            ("Email", "email", "email")
        };

    fill_consent(
        &with_hosted_page_css(LOGIN_TEMPLATE),
        !consent_required,
        target,
    )
    .replace(
        "{{CONSENT_REQUIRED}}",
        if consent_required { "true" } else { "false" },
    )
    .replace("{{ID_LABEL}}", id_label)
    .replace("{{ID_TYPE}}", id_type)
    .replace("{{ID_AUTOCOMPLETE}}", id_autocomplete)
    .replace("{{LINK_TOKEN}}", &escape_html_attribute(link_token))
    .replace("{{TARGET}}", &escape_html_attribute(target))
    .replace("{{TARGET_LABEL}}", &escape_html_attribute(target_label))
    .replace("{{CHANNEL}}", &escape_html_attribute(channel))
    .replace("{{CHANNEL_LABEL}}", &escape_html_attribute(&channel_label))
}

/// Render the success page shown after a successful connection. Reused by the
/// hosted connect picker, so the label map covers OAuth providers too.
#[must_use]
pub fn render_success_page(channel: &str, target: &str) -> String {
    let target_label =
        backend_resolver::brand_name(&global_registry(), target).unwrap_or("fitness");
    let channel_label = humanize_channel(channel);

    with_hosted_page_css(SUCCESS_TEMPLATE)
        .replace("{{TARGET_LABEL}}", &escape_html_attribute(target_label))
        .replace("{{CHANNEL}}", &escape_html_attribute(channel))
        .replace("{{CHANNEL_LABEL}}", &escape_html_attribute(&channel_label))
}

/// Render the error page for invalid/expired links or other hosted-login failures.
#[must_use]
pub fn render_error_page(message: &str) -> String {
    with_hosted_page_css(ERROR_TEMPLATE).replace("{{MESSAGE}}", &escape_html_attribute(message))
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
