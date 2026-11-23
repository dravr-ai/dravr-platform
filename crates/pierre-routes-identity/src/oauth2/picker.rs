// ABOUTME: What the consent screen offers to grant — Everything as today, a preset, or a request's own resolved details
// ABOUTME: Each choice carries the review digest it was rendered under; a submission whose digest no longer holds is refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The grant picker and the review digest (owl PDPP §3.3, §7.4)
//!
//! A request with no `authorization_details`, which no remembered grant
//! answers, is shown three choices: **Everything, as today** (a plain grant,
//! preselected, so an athlete who clicks Allow keeps today's access),
//! **Training coach** and **Sleep coach**. The Admin access box sits beside
//! Everything alone: a preset grant never carries `admin`.
//!
//! A request with details is shown what they resolved to, with the client's
//! purpose and commitments quoted as its own words.
//!
//! Every choice carries its review digest: the consent text version, and the
//! terms of the details it grants ([`review_digest`]). The submission
//! recomputes it and refuses a form whose digest no longer holds, so what the
//! athlete approves is what the screen showed. A plain grant's digest is the
//! text version itself.
//!
//! **The lines are not in the catalogue yet.** They are authored in
//! dravr-contremaitre first and reach this crate with the pin. Until every
//! one of them reads ([`offered`]), the screen shows the plain grant alone,
//! as before, and a request carrying details is refused before sign-in: its
//! details are never shown with empty lines, nor granted unseen.

use std::collections::HashMap;
use std::fmt::Write as _;

use pierre_auth::oauth2_server::authorization_details::{
    preset_defaults, review_digest, Preset, Representation, ResolvedDetails,
};
use pierre_auth::oauth2_server::endpoints::CheckedAuthorization;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_contremaitre::consent_strings::{
    GRANT_KEY_CHOICE_TITLE, GRANT_KEY_COMMITMENTS, GRANT_KEY_EVERYTHING, GRANT_KEY_EVERYTHING_HINT,
    GRANT_KEY_LASTS_DAYS, GRANT_KEY_LASTS_HOURS, GRANT_KEY_PURPOSE, GRANT_KEY_REQUESTED,
    GRANT_KEY_SLEEP_COACH, GRANT_KEY_SLEEP_COACH_HINT, GRANT_KEY_TRAINING_COACH,
    GRANT_KEY_TRAINING_COACH_HINT, GRANT_KEY_TRAINING_COACH_SUMMARY_HINT,
};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::time_constants;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::OAuthClientGrant;
use tracing::{error, warn};

use super::consent_text::ConsentText;
use super::consent_version::{consent_version, ConsentSubject};

/// Every line the picker and the requested details show.
const GRANT_KEYS: [&str; 13] = [
    GRANT_KEY_CHOICE_TITLE,
    GRANT_KEY_EVERYTHING,
    GRANT_KEY_EVERYTHING_HINT,
    GRANT_KEY_TRAINING_COACH,
    GRANT_KEY_TRAINING_COACH_HINT,
    GRANT_KEY_TRAINING_COACH_SUMMARY_HINT,
    GRANT_KEY_SLEEP_COACH,
    GRANT_KEY_SLEEP_COACH_HINT,
    GRANT_KEY_LASTS_DAYS,
    GRANT_KEY_LASTS_HOURS,
    GRANT_KEY_REQUESTED,
    GRANT_KEY_PURPOSE,
    GRANT_KEY_COMMITMENTS,
];

/// The picker's form field, and the value of its plain choice.
const FIELD_GRANT: &str = "grant";
const EVERYTHING: &str = "everything";

/// The form field carrying a plain screen's or a request's review digest;
/// the picker's choices each carry theirs as `review_digest_<choice>`.
const FIELD_DIGEST: &str = "review_digest";

/// The refusal of an approval whose screen changed while it was open.
pub(super) const CONSENT_CHANGED: &str =
    "The consent screen changed while it was open; start the connection again";

/// Whether the catalogue carries every line the picker and a request's
/// details show, in English at least: until it does, neither is shown.
pub(super) fn offered(strings: &MessagingStringsRegistry) -> bool {
    let english = ConsentText::new(strings, "en");
    GRANT_KEYS.iter().all(|key| !english.line(key).is_empty())
}

/// Refuse a request carrying details the screen cannot show yet.
///
/// # Errors
/// `invalid_authorization_details` while the lines are not in the catalogue.
pub(super) fn refuse_unoffered_details(
    strings: &MessagingStringsRegistry,
    checked: &CheckedAuthorization,
) -> Result<(), OAuth2Error> {
    if checked.details.is_some() && !offered(strings) {
        warn!("authorization_details refused: the consent screen cannot show them yet");
        return Err(OAuth2Error::invalid_authorization_details(
            "authorization_details are not offered yet",
        ));
    }
    Ok(())
}

/// One of the picker's choices.
#[derive(Debug, Clone)]
struct Choice {
    /// The form value: `everything` or a preset's name
    value: &'static str,
    /// What it grants; `None` is the plain grant
    details: Option<ResolvedDetails>,
    review_digest: String,
}

#[derive(Debug, Clone)]
enum Offer {
    Plain {
        review_digest: String,
    },
    Picker {
        choices: Vec<Choice>,
    },
    Requested {
        details: Box<ResolvedDetails>,
        raw: String,
        review_digest: String,
    },
}

/// What the consent screen offers the athlete to grant, each choice with the
/// review digest it is rendered under.
#[derive(Debug, Clone)]
pub struct GrantChoice(Offer);

impl GrantChoice {
    /// What the screen for `checked` offers, read in `text` for `subject`.
    /// `raw` is the request's `authorization_details` parameter, echoed by a
    /// form whose request was not pushed.
    pub(super) fn for_screen(
        checked: &CheckedAuthorization,
        raw: Option<&str>,
        text: &ConsentText<'_>,
        subject: ConsentSubject<'_>,
    ) -> Result<Self, OAuth2Error> {
        let version = consent_version(text, subject);
        let digest = |details: Option<&ResolvedDetails>| {
            review_digest(&version, details).map_err(|e| {
                error!("The consent screen's review digest could not be computed: {e}");
                OAuth2Error::server_error(super::consent::CONSENT_UNAVAILABLE)
            })
        };
        if let Some(details) = &checked.details {
            return Ok(Self(Offer::Requested {
                details: Box::new(details.clone()),
                raw: raw.unwrap_or_default().to_owned(),
                review_digest: digest(Some(details))?,
            }));
        }
        let Some(source) = checked
            .preset_source
            .as_deref()
            .filter(|_| offered(text.strings()))
        else {
            return Ok(Self(Offer::Plain {
                review_digest: digest(None)?,
            }));
        };
        let mut choices = vec![Choice {
            value: EVERYTHING,
            details: None,
            review_digest: digest(None)?,
        }];
        for preset in Preset::ALL {
            let details = preset_defaults(preset, source);
            choices.push(Choice {
                value: preset.name(),
                review_digest: digest(Some(&details))?,
                details: Some(details),
            });
        }
        Ok(Self(Offer::Picker { choices }))
    }

    /// The choice's markup: the picker places `admin_box` beside
    /// Everything; a request's own details never show it.
    pub(super) fn render(&self, admin_box: &str, client: &str, text: &ConsentText<'_>) -> String {
        match &self.0 {
            Offer::Plain { review_digest } => hidden(FIELD_DIGEST, review_digest),
            Offer::Requested {
                details,
                raw,
                review_digest,
            } => requested_html(details, raw, review_digest, client, text),
            Offer::Picker { choices } => picker_html(choices, admin_box, client, text),
        }
    }

    /// Whether the choice decides where the Admin access box goes, so the
    /// page places none of its own: the picker shows it beside Everything,
    /// and a request's own details never show it.
    pub(super) const fn places_admin_box(&self) -> bool {
        !matches!(self.0, Offer::Plain { .. })
    }
}

/// A hidden form field.
fn hidden(name: &str, value: &str) -> String {
    format!(
        r#"<input type="hidden" name="{}" value="{}">"#,
        escape_html_attribute(name),
        escape_html_attribute(value)
    )
}

/// The field carrying `choice`'s digest on the picker.
fn picker_digest_field(choice: &str) -> String {
    format!("{FIELD_DIGEST}_{choice}")
}

/// A preset's name and what it reads, in `text`'s language.
fn preset_lines(details: &ResolvedDetails, text: &ConsentText<'_>) -> (String, String) {
    let days = details.window_days().to_string();
    let (name, hint) = match details.preset() {
        Preset::SleepCoach => (GRANT_KEY_SLEEP_COACH, GRANT_KEY_SLEEP_COACH_HINT),
        _ if details.representation() == Representation::Summary => (
            GRANT_KEY_TRAINING_COACH,
            GRANT_KEY_TRAINING_COACH_SUMMARY_HINT,
        ),
        _ => (GRANT_KEY_TRAINING_COACH, GRANT_KEY_TRAINING_COACH_HINT),
    };
    (
        text.html(name, &[]),
        format!(
            "{} {}",
            text.html(hint, &[&days]),
            lasts_line(details, text)
        ),
    )
}

/// How long a preset grant lasts: whole days, else hours rounded up.
fn lasts_line(details: &ResolvedDetails, text: &ConsentText<'_>) -> String {
    let seconds = details.lifetime().num_seconds();
    let day = i64::from(time_constants::SECONDS_PER_DAY);
    if seconds % day == 0 {
        text.html(GRANT_KEY_LASTS_DAYS, &[&(seconds / day).to_string()])
    } else {
        let hour = i64::from(time_constants::SECONDS_PER_HOUR);
        let hours = (seconds + hour - 1) / hour;
        text.html(GRANT_KEY_LASTS_HOURS, &[&hours.to_string()])
    }
}

/// The picker: Everything preselected with the Admin access box beside it,
/// then each preset, and every choice's digest.
fn picker_html(
    choices: &[Choice],
    admin_box: &str,
    client: &str,
    text: &ConsentText<'_>,
) -> String {
    let mut html = format!(
        r#"<fieldset class="grant-choice"><legend class="scopes-title">{}</legend>"#,
        text.html(GRANT_KEY_CHOICE_TITLE, &[client])
    );
    for choice in choices {
        let (label, hint, checked) = choice.details.as_ref().map_or_else(
            || {
                (
                    text.html(GRANT_KEY_EVERYTHING, &[]),
                    text.html(GRANT_KEY_EVERYTHING_HINT, &[]),
                    " checked",
                )
            },
            |details| {
                let (label, hint) = preset_lines(details, text);
                (label, hint, "")
            },
        );
        write!(
            html,
            r#"<label class="check"><input type="radio" name="{FIELD_GRANT}" value="{value}"{checked}> <span class="callout-title">{label}</span></label><p class="fineprint">{hint}</p>"#,
            value = choice.value,
        )
        .ok();
        if choice.details.is_none() {
            html.push_str(admin_box);
        }
        html.push_str(&hidden(
            &picker_digest_field(choice.value),
            &choice.review_digest,
        ));
    }
    html.push_str("</fieldset>");
    html
}

/// A request's own details as they resolved: the preset, what it reads and
/// for how long, and the client's purpose and commitments in its own words.
fn requested_html(
    details: &ResolvedDetails,
    raw: &str,
    review_digest: &str,
    client: &str,
    text: &ConsentText<'_>,
) -> String {
    let (label, hint) = preset_lines(details, text);
    let mut html = format!(
        r#"<div class="callout"><p class="callout-title">{} {label}</p><p>{hint}</p>"#,
        text.html(GRANT_KEY_REQUESTED, &[client]),
    );
    if let Some(purpose) = details.purpose_description() {
        write!(
            html,
            "<p>{}</p>",
            text.html(
                GRANT_KEY_PURPOSE,
                &[client, &escape_html_attribute(purpose)]
            )
        )
        .ok();
    }
    if !details.commitments().is_empty() {
        write!(
            html,
            "<p>{}</p><ul>",
            text.html(GRANT_KEY_COMMITMENTS, &[client])
        )
        .ok();
        for commitment in details.commitments() {
            write!(
                html,
                "<li><q>{}</q></li>",
                escape_html_attribute(commitment)
            )
            .ok();
        }
        html.push_str("</ul>");
    }
    html.push_str(&hidden("authorization_details", raw));
    html.push_str(&hidden(FIELD_DIGEST, review_digest));
    html.push_str("</div>");
    html
}

/// The details the athlete approved with `form` for `checked`: a request's
/// own, a preset the picker offered, or `None` for the plain grant; and the
/// field the screen carried their digest in.
pub(super) fn chosen(
    form: &HashMap<String, String>,
    checked: &CheckedAuthorization,
    strings: &MessagingStringsRegistry,
) -> (Option<ResolvedDetails>, String) {
    if let Some(details) = &checked.details {
        return (Some(details.clone()), FIELD_DIGEST.to_owned());
    }
    let Some(source) = checked
        .preset_source
        .as_deref()
        .filter(|_| offered(strings))
    else {
        return (None, FIELD_DIGEST.to_owned());
    };
    form.get(FIELD_GRANT)
        .and_then(|choice| Preset::from_name(choice))
        .map_or_else(
            || (None, picker_digest_field(EVERYTHING)),
            |preset| {
                (
                    Some(preset_defaults(preset, source)),
                    picker_digest_field(preset.name()),
                )
            },
        )
}

/// Refuse an approval whose screen no longer reads as it did: the digest the
/// form carries in `field` must be the one `details` have under the text the
/// athlete reads now.
///
/// # Errors
/// `access_denied` when the digest differs or is missing; `server_error`
/// when it cannot be computed.
pub(super) fn check_review_digest(
    form: &HashMap<String, String>,
    field: &str,
    details: Option<&ResolvedDetails>,
    text: &ConsentText<'_>,
    subject: ConsentSubject<'_>,
) -> Result<String, OAuth2Error> {
    let expected = review_digest(&consent_version(text, subject), details).map_err(|e| {
        error!("The consent submission's review digest could not be computed: {e}");
        OAuth2Error::server_error(super::consent::CONSENT_NOT_RECORDED)
    })?;
    if form.get(field) != Some(&expected) {
        warn!("OAuth consent refused: the screen changed since it was rendered");
        return Err(OAuth2Error::access_denied(CONSENT_CHANGED));
    }
    Ok(expected)
}

/// The details `grant` stored; `None` for a plain grant, or one whose
/// document does not read (logged), which no request then matches.
pub(super) fn stored_details(grant: &OAuthClientGrant) -> Result<Option<ResolvedDetails>, ()> {
    grant
        .authorization_details
        .as_deref()
        .map(ResolvedDetails::from_document)
        .transpose()
        .map_err(
            |e| error!(grant_id = %grant.id, "A grant's authorization details do not read: {e}"),
        )
}

/// Whether a live `grant` answers `checked` without the screen: it grants
/// the request's details (any grant answers a request naming none) and was
/// approved under the text the athlete reads now.
pub(super) fn skip_accepts(
    strings: &MessagingStringsRegistry,
    grant: &OAuthClientGrant,
    checked: &CheckedAuthorization,
    subject: ConsentSubject<'_>,
) -> bool {
    let Ok(stored) = stored_details(grant) else {
        return false;
    };
    if let Some(asked) = &checked.details {
        if !stored
            .as_ref()
            .is_some_and(|stored| stored.same_terms(asked))
        {
            return false;
        }
    }
    super::consent_version::approved_under_current_text(strings, grant, stored.as_ref(), subject)
}
