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
//! A picker whose lines read empty in English is not shown ([`offered`]):
//! the screen then offers the plain grant alone, and an athlete is never
//! handed an unlabelled choice.
//!
//! Beside a preset choice, never the plain one and never for an admin, the
//! screen may offer the standing-policy box, "Always allow this app this
//! preset" ([`offers_standing_policy_box`]): ticked, the approval also stores a
//! standing policy that renews the same grant without the screen. Connected
//! apps lists the athlete's policies with a way to remove each, and a box
//! field the screen did not offer is ignored ([`standing_policy_ticked`]).

use std::collections::HashMap;
use std::fmt::Write as _;

use pierre_auth::oauth2_server::authorization_details::{
    preset_defaults, review_digest, GrantDocument, Preset, Representation, ResolvedDetails, Stream,
};
use pierre_auth::oauth2_server::endpoints::CheckedAuthorization;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_contremaitre::consent_strings::step_up_worded;
use pierre_contremaitre::consent_strings::{
    KEY_CONSENT_GRANT_CHOICE_TITLE, KEY_CONSENT_GRANT_COMMITMENTS, KEY_CONSENT_GRANT_EVERYTHING,
    KEY_CONSENT_GRANT_EVERYTHING_HINT, KEY_CONSENT_GRANT_LASTS_DAYS, KEY_CONSENT_GRANT_LASTS_HOURS,
    KEY_CONSENT_GRANT_PURPOSE, KEY_CONSENT_GRANT_REQUESTED, KEY_CONSENT_GRANT_SLEEP_COACH,
    KEY_CONSENT_GRANT_SLEEP_COACH_HINT, KEY_CONSENT_GRANT_TRAINING_COACH,
    KEY_CONSENT_GRANT_TRAINING_COACH_HINT, KEY_CONSENT_GRANT_TRAINING_COACH_SUMMARY_HINT,
    KEY_CONSENT_STANDING_POLICY, KEY_STEP_UP_HOST, KEY_STEP_UP_STREAMS_TITLE,
    KEY_STEP_UP_STREAM_ACTIVITY_DAILY, KEY_STEP_UP_STREAM_BODY, KEY_STEP_UP_STREAM_CARDIO_DAILY,
    KEY_STEP_UP_STREAM_FITNESS, KEY_STEP_UP_STREAM_LABS, KEY_STEP_UP_STREAM_PROFILE,
    KEY_STEP_UP_STREAM_RECOVERY_DAILY, KEY_STEP_UP_STREAM_SLEEP, KEY_STEP_UP_STREAM_TRAINING_LOAD,
    KEY_STEP_UP_STREAM_WORKOUT, KEY_STEP_UP_T2_DISCLOSURE, KEY_STEP_UP_TITLE,
};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::constants::time_constants;
use pierre_core::html::escape_html_attribute;
use pierre_core::models::{OAuthClientGrant, User};
use tracing::{error, warn};

use super::consent_text::ConsentText;
use super::consent_version::{consent_version, screen_version, ConsentSubject};

/// Every line the picker and the requested details show.
const GRANT_KEYS: [&str; 13] = [
    KEY_CONSENT_GRANT_CHOICE_TITLE,
    KEY_CONSENT_GRANT_EVERYTHING,
    KEY_CONSENT_GRANT_EVERYTHING_HINT,
    KEY_CONSENT_GRANT_TRAINING_COACH,
    KEY_CONSENT_GRANT_TRAINING_COACH_HINT,
    KEY_CONSENT_GRANT_TRAINING_COACH_SUMMARY_HINT,
    KEY_CONSENT_GRANT_SLEEP_COACH,
    KEY_CONSENT_GRANT_SLEEP_COACH_HINT,
    KEY_CONSENT_GRANT_LASTS_DAYS,
    KEY_CONSENT_GRANT_LASTS_HOURS,
    KEY_CONSENT_GRANT_REQUESTED,
    KEY_CONSENT_GRANT_PURPOSE,
    KEY_CONSENT_GRANT_COMMITMENTS,
];

/// The picker's form field, and the value of its plain choice.
const FIELD_GRANT: &str = "grant";
const EVERYTHING: &str = "everything";

/// The form field carrying a plain screen's or a request's review digest;
/// the picker's choices each carry theirs as `review_digest_<choice>`.
const FIELD_DIGEST: &str = "review_digest";

/// The form field of a request's standing-policy box; the picker's presets
/// each carry theirs as `standing_policy_<preset>`.
const FIELD_STANDING_POLICY: &str = "standing_policy";

/// The step-up screen's checkbox of one stream: this prefix, then the
/// stream's owl name (`step_up_stream:labs.summary`). Each its own field: a
/// form posts as one value per name.
const FIELD_STEP_UP_STREAM: &str = "step_up_stream:";

/// The refusal of a step-up approved with every stream unticked.
const STEP_UP_NO_STREAM: &str = "choose at least one kind of data to share";

/// The refusal of a step-up approval ticking a stream the request did not
/// ask for.
const STEP_UP_STREAM_NOT_ASKED: &str = "the approval names data the application did not ask for";

/// Whether `user`'s consent screen may offer the standing-policy box: never
/// to an admin, whose access is chosen per connection. The screen and the
/// submit both read it, so a box the screen never offered is never read from
/// a form.
pub(super) const fn offers_standing_policy_box(user: &User) -> bool {
    !user.role.is_admin()
}

/// The refusal of an approval whose screen changed while it was open.
pub(super) const CONSENT_CHANGED: &str =
    "The consent screen changed while it was open; start the connection again";

/// Whether the catalogue carries every line the picker shows, in English at
/// least: a picker with an empty line is not shown.
pub(super) fn offered(strings: &MessagingStringsRegistry) -> bool {
    let english = ConsentText::new(strings, "en");
    GRANT_KEYS.iter().all(|key| !english.line(key).is_empty())
}

/// Whether the catalogue words the step-up screen, in English at least: a
/// step-up request is resolved only then (carnet#1544 slice 4b), and refused
/// before sign-in otherwise.
pub(super) fn step_up_offered(strings: &MessagingStringsRegistry) -> bool {
    offered(strings) && step_up_worded(strings)
}

/// The catalogue line whose presence offers agent-write grants: the write
/// section's title on the consent screen (carnet#1544 slice 4c).
pub(super) const KEY_AGENT_WRITE_TITLE: &str = "oauth.agentWrite.title";

/// Whether the catalogue words an agent-write consent, in English at least.
///
/// An agent-write request is resolved only then, and refused before sign-in "agent writes are
/// not offered yet" otherwise (`consent-agent-write` in `feature-phases.yaml`).
pub(super) fn agent_write_offered(strings: &MessagingStringsRegistry) -> bool {
    !ConsentText::new(strings, "en")
        .line(KEY_AGENT_WRITE_TITLE)
        .is_empty()
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
/// review digest it is rendered under, and whether a preset choice carries
/// the standing-policy box.
#[derive(Debug, Clone)]
pub struct GrantChoice {
    offer: Offer,
    standing_policy_box: bool,
}

impl GrantChoice {
    /// What the screen for `checked` offers, read in `text` for `subject`.
    /// `raw` is the request's `authorization_details` parameter, echoed by a
    /// form whose request was not pushed. `standing_policy_box` places the
    /// box beside each preset choice: for anyone but an admin
    /// ([`offers_standing_policy_box`]), and only while the catalogue words it.
    pub(super) fn for_screen(
        checked: &CheckedAuthorization,
        raw: Option<&str>,
        text: &ConsentText<'_>,
        subject: ConsentSubject<'_>,
        standing_policy_box: bool,
    ) -> Result<Self, OAuth2Error> {
        let standing_policy_box =
            standing_policy_box && !text.line(KEY_CONSENT_STANDING_POLICY).is_empty();
        let offer = Self::offer(checked, raw, text, subject)?;
        Ok(Self {
            offer,
            standing_policy_box,
        })
    }

    /// What the screen for `checked` offers to grant.
    fn offer(
        checked: &CheckedAuthorization,
        raw: Option<&str>,
        text: &ConsentText<'_>,
        subject: ConsentSubject<'_>,
    ) -> Result<Offer, OAuth2Error> {
        let version = consent_version(text, subject);
        let digest = |details: Option<&ResolvedDetails>| {
            review_digest(&screen_version(&version, text, details), details).map_err(|e| {
                error!("The consent screen's review digest could not be computed: {e}");
                OAuth2Error::server_error(super::consent::CONSENT_UNAVAILABLE)
            })
        };
        if let Some(details) = &checked.details {
            return Ok(Offer::Requested {
                details: Box::new(details.clone()),
                raw: raw.unwrap_or_default().to_owned(),
                review_digest: digest(Some(details))?,
            });
        }
        let Some(source) = checked
            .preset_source
            .as_deref()
            .filter(|_| offered(text.strings()))
        else {
            return Ok(Offer::Plain {
                review_digest: digest(None)?,
            });
        };
        let mut choices = vec![Choice {
            value: EVERYTHING,
            details: None,
            review_digest: digest(None)?,
        }];
        for preset in ordinary_presets() {
            let details = preset_defaults(preset, source);
            choices.push(Choice {
                value: preset.name(),
                review_digest: digest(Some(&details))?,
                details: Some(details),
            });
        }
        Ok(Offer::Picker { choices })
    }

    /// The choice's markup: the picker places `admin_box` beside
    /// Everything; a request's own details never show it.
    pub(super) fn render(&self, admin_box: &str, client: &str, text: &ConsentText<'_>) -> String {
        let policy_box = |field: &str| {
            if self.standing_policy_box {
                standing_policy_box_html(field, client, text)
            } else {
                String::new()
            }
        };
        match &self.offer {
            Offer::Plain { review_digest } => hidden(FIELD_DIGEST, review_digest),
            Offer::Requested {
                details,
                raw,
                review_digest,
            } if details.preset().requires_step_up() => {
                step_up_html(details, raw, review_digest, client, text)
            }
            Offer::Requested {
                details,
                raw,
                review_digest,
            } => requested_html(
                details,
                raw,
                review_digest,
                client,
                text,
                &policy_box(FIELD_STANDING_POLICY),
            ),
            Offer::Picker { choices } => picker_html(choices, admin_box, client, text, &policy_box),
        }
    }

    /// Whether the choice decides where the Admin access box goes, so the
    /// page places none of its own: the picker shows it beside Everything,
    /// and a request's own details never show it.
    pub(super) const fn places_admin_box(&self) -> bool {
        !matches!(self.offer, Offer::Plain { .. })
    }

    /// Whether the screen grants one preset alone: a request's own details,
    /// which always narrow a preset. The picker offers Everything too.
    pub(super) const fn is_preset(&self) -> bool {
        matches!(self.offer, Offer::Requested { .. })
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

/// The field carrying the standing-policy box of preset `choice` on the
/// picker.
fn picker_policy_field(choice: &str) -> String {
    format!("{FIELD_STANDING_POLICY}_{choice}")
}

/// The standing-policy box, unticked, named `field`: always allow `client`
/// this preset.
fn standing_policy_box_html(field: &str, client: &str, text: &ConsentText<'_>) -> String {
    format!(
        r#"<label class="check standing-policy"><input type="checkbox" name="{}" value="on"> <span>{}</span></label>"#,
        escape_html_attribute(field),
        text.html(KEY_CONSENT_STANDING_POLICY, &[client])
    )
}

/// The presets ordinary consent offers: every one but a step-up preset
/// (`one_off_question`), which is granted only on the step-up screen as a
/// child of a live grant (carnet#1544 slice 4b, review R-M5).
fn ordinary_presets() -> impl Iterator<Item = Preset> {
    Preset::ALL
        .into_iter()
        .filter(|preset| !preset.requires_step_up())
}

/// A preset's name and what it reads, in `text`'s language.
fn preset_lines(details: &ResolvedDetails, text: &ConsentText<'_>) -> (String, String) {
    let days = details.window_days().to_string();
    let (name, hint) = match details.preset() {
        Preset::SleepCoach => (
            KEY_CONSENT_GRANT_SLEEP_COACH,
            KEY_CONSENT_GRANT_SLEEP_COACH_HINT,
        ),
        _ if details.representation() == Representation::Summary => (
            KEY_CONSENT_GRANT_TRAINING_COACH,
            KEY_CONSENT_GRANT_TRAINING_COACH_SUMMARY_HINT,
        ),
        _ => (
            KEY_CONSENT_GRANT_TRAINING_COACH,
            KEY_CONSENT_GRANT_TRAINING_COACH_HINT,
        ),
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
        text.html(
            KEY_CONSENT_GRANT_LASTS_DAYS,
            &[&(seconds / day).to_string()],
        )
    } else {
        let hour = i64::from(time_constants::SECONDS_PER_HOUR);
        let hours = (seconds + hour - 1) / hour;
        text.html(KEY_CONSENT_GRANT_LASTS_HOURS, &[&hours.to_string()])
    }
}

/// The picker: Everything preselected with the Admin access box beside it,
/// then each preset with the standing-policy box `policy_box` makes for its
/// field (empty when the screen offers none), and every choice's digest.
fn picker_html(
    choices: &[Choice],
    admin_box: &str,
    client: &str,
    text: &ConsentText<'_>,
    policy_box: &dyn Fn(&str) -> String,
) -> String {
    let mut html = format!(
        r#"<fieldset class="scopes grant-choice"><legend class="scopes-title">{}</legend>"#,
        text.html(KEY_CONSENT_GRANT_CHOICE_TITLE, &[client])
    );
    for choice in choices {
        let (label, hint, checked) = choice.details.as_ref().map_or_else(
            || {
                (
                    text.html(KEY_CONSENT_GRANT_EVERYTHING, &[]),
                    text.html(KEY_CONSENT_GRANT_EVERYTHING_HINT, &[]),
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
            r#"<div class="grant-option"><label class="check"><input type="radio" name="{FIELD_GRANT}" value="{value}"{checked}> <span class="grant-label">{label}</span></label><p class="grant-hint">{hint}</p>"#,
            value = choice.value,
        )
        .ok();
        if choice.details.is_none() {
            html.push_str(admin_box);
        } else {
            html.push_str(&policy_box(&picker_policy_field(choice.value)));
        }
        html.push_str(&hidden(
            &picker_digest_field(choice.value),
            &choice.review_digest,
        ));
        html.push_str("</div>");
    }
    html.push_str("</fieldset>");
    html
}

/// A request's own details as they resolved: the preset, what it reads and
/// for how long, the client's purpose and commitments in its own words, and
/// the standing-policy box `policy_box` (empty when the screen offers none).
fn requested_html(
    details: &ResolvedDetails,
    raw: &str,
    review_digest: &str,
    client: &str,
    text: &ConsentText<'_>,
    policy_box: &str,
) -> String {
    let (label, hint) = preset_lines(details, text);
    let mut html = format!(
        r#"<div class="scopes grant-requested"><p class="grant-label">{} {label}</p><p>{hint}</p>"#,
        text.html(KEY_CONSENT_GRANT_REQUESTED, &[client]),
    );
    if let Some(purpose) = details.purpose_description() {
        write!(
            html,
            "<p>{}</p>",
            text.html(
                KEY_CONSENT_GRANT_PURPOSE,
                &[client, &escape_html_attribute(purpose)]
            )
        )
        .ok();
    }
    if !details.commitments().is_empty() {
        write!(
            html,
            "<p>{}</p><ul>",
            text.html(KEY_CONSENT_GRANT_COMMITMENTS, &[client])
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
    html.push_str(policy_box);
    html.push_str(&hidden("authorization_details", raw));
    html.push_str(&hidden(FIELD_DIGEST, review_digest));
    html.push_str("</div>");
    html
}

/// The step-up screen's details: a one-off look that includes health
/// records (T2), read once and then ended, how long it may wait to be used,
/// the host's own unverifiable declaration, the streams asked for, each a
/// checkbox ticked by default ([`ticked_details`]), and the client's purpose
/// in its own words. No standing-policy box: a policy never renews a T2 grant (owl
/// §6.4).
fn step_up_html(
    details: &ResolvedDetails,
    raw: &str,
    review_digest: &str,
    client: &str,
    text: &ConsentText<'_>,
) -> String {
    let mut html = format!(
        r#"<div class="scopes grant-requested step-up"><p class="grant-label">{}</p><p>{} {}</p><p class="step-up-host">{}</p>"#,
        text.html(KEY_STEP_UP_TITLE, &[client]),
        text.html(KEY_STEP_UP_T2_DISCLOSURE, &[client]),
        lasts_line(details, text),
        text.html(KEY_STEP_UP_HOST, &[client]),
    );
    let streams = details.picked_streams();
    if !streams.is_empty() {
        write!(
            html,
            r#"<p class="grant-label">{}</p><ul class="step-up-streams">"#,
            text.html(KEY_STEP_UP_STREAMS_TITLE, &[client])
        )
        .ok();
        for stream in streams {
            // A stream with no label is named as owl names it: the screen
            // never hides what the request reads.
            let label = step_up_stream_key(stream)
                .map(|key| text.html(key, &[]))
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| escape_html_attribute(stream.name()));
            // Ticked by default: the owner unticks what the client should
            // not read (owl §3.1), and the approval refuses none ticked.
            write!(
                html,
                r#"<li><label><input type="checkbox" name="{}{}" value="on" checked> {label}</label></li>"#,
                FIELD_STEP_UP_STREAM,
                escape_html_attribute(stream.name()),
            )
            .ok();
        }
        html.push_str("</ul>");
    }
    if let Some(purpose) = details.purpose_description() {
        write!(
            html,
            "<p>{}</p>",
            text.html(
                KEY_CONSENT_GRANT_PURPOSE,
                &[client, &escape_html_attribute(purpose)]
            )
        )
        .ok();
    }
    html.push_str(&hidden("authorization_details", raw));
    html.push_str(&hidden(FIELD_DIGEST, review_digest));
    html.push_str("</div>");
    html
}

/// The one-off question the athlete approved on the step-up screen:
/// `details`, the request's, narrowed to the streams `form` left ticked.
///
/// # Errors
/// `access_denied` when none is ticked, or one the request did not ask for
/// is: nothing is dropped or added silently.
pub(super) fn ticked_details(
    form: &HashMap<String, String>,
    details: &ResolvedDetails,
) -> Result<ResolvedDetails, OAuth2Error> {
    let mut ticked = Vec::new();
    for name in form
        .keys()
        .filter_map(|key| key.strip_prefix(FIELD_STEP_UP_STREAM))
    {
        let stream = Stream::from_name(name)
            .filter(|stream| details.picked_streams().contains(stream))
            .ok_or_else(|| {
                warn!(
                    stream = name,
                    "OAuth consent refused: a step-up ticked a stream not asked for"
                );
                OAuth2Error::access_denied(STEP_UP_STREAM_NOT_ASKED)
            })?;
        ticked.push(stream);
    }
    if ticked.is_empty() {
        return Err(OAuth2Error::access_denied(STEP_UP_NO_STREAM));
    }
    details.narrowed_to(&ticked).map_err(|e| {
        warn!("OAuth consent refused: the step-up selection does not narrow the request: {e}");
        OAuth2Error::access_denied(STEP_UP_STREAM_NOT_ASKED)
    })
}

/// The one-off question the athlete approved, and the digest the child records.
///
/// [`ticked_details`], then [`selection_digest`] of what it returns.
///
/// # Errors
/// As either refuses.
pub(super) fn step_up_selection(
    form: &HashMap<String, String>,
    details: &ResolvedDetails,
    text: &ConsentText<'_>,
    subject: ConsentSubject<'_>,
) -> Result<(ResolvedDetails, String), OAuth2Error> {
    let selected = ticked_details(form, details)?;
    let digest = selection_digest(&selected, text, subject)?;
    Ok((selected, digest))
}

/// The review digest of `details`, narrowed by the athlete on the step-up
/// screen, under the text that screen was rendered in: the digest the child
/// records, so it covers the selection the athlete approved.
///
/// # Errors
/// `server_error` when the digest cannot be computed.
pub(super) fn selection_digest(
    details: &ResolvedDetails,
    text: &ConsentText<'_>,
    subject: ConsentSubject<'_>,
) -> Result<String, OAuth2Error> {
    let version = consent_version(text, subject);
    review_digest(
        &screen_version(&version, text, Some(details)),
        Some(details),
    )
    .map_err(|e| {
        error!("The step-up selection's review digest could not be computed: {e}");
        OAuth2Error::server_error(super::consent::CONSENT_NOT_RECORDED)
    })
}

/// The step-up screen's label of `stream`, one of those a one-off question
/// may pick; `None` for any other.
const fn step_up_stream_key(stream: Stream) -> Option<&'static str> {
    Some(match stream {
        Stream::ActivityDaily => KEY_STEP_UP_STREAM_ACTIVITY_DAILY,
        Stream::Workout => KEY_STEP_UP_STREAM_WORKOUT,
        Stream::TrainingLoad => KEY_STEP_UP_STREAM_TRAINING_LOAD,
        Stream::Sleep => KEY_STEP_UP_STREAM_SLEEP,
        Stream::CardioDaily => KEY_STEP_UP_STREAM_CARDIO_DAILY,
        Stream::Body => KEY_STEP_UP_STREAM_BODY,
        Stream::Fitness => KEY_STEP_UP_STREAM_FITNESS,
        Stream::RecoveryDaily => KEY_STEP_UP_STREAM_RECOVERY_DAILY,
        Stream::Profile => KEY_STEP_UP_STREAM_PROFILE,
        Stream::Labs => KEY_STEP_UP_STREAM_LABS,
        _ => return None,
    })
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
    // Only a preset the picker offers: a step-up preset is granted on the
    // step-up screen alone, never from a picker form naming it.
    form.get(FIELD_GRANT)
        .and_then(|choice| Preset::from_name(choice))
        .filter(|preset| !preset.requires_step_up())
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

/// Whether the athlete ticked the standing-policy box beside the preset they
/// approved with `form` for `checked`, the screen read in `text`. Never
/// while the box is not offered (`offered_box`: [`offers_standing_policy_box`],
/// anyone but an admin; and `text` words it), and never on the plain grant:
/// a forged field is ignored.
pub(super) fn standing_policy_ticked(
    form: &HashMap<String, String>,
    checked: &CheckedAuthorization,
    text: &ConsentText<'_>,
    offered_box: bool,
) -> bool {
    if !offered_box || text.line(KEY_CONSENT_STANDING_POLICY).is_empty() {
        return false;
    }
    let field = if checked.details.is_some() {
        FIELD_STANDING_POLICY.to_owned()
    } else {
        match chosen(form, checked, text.strings()).0 {
            Some(details) => picker_policy_field(details.preset().name()),
            None => return false,
        }
    };
    form.get(&field).is_some_and(|ticked| ticked == "on")
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
    let version = consent_version(text, subject);
    let expected =
        review_digest(&screen_version(&version, text, details), details).map_err(|e| {
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
///
/// An agent-write grant's document is not a read grant's details: it reads
/// as `Err`, so no read request is ever answered by it, nor compared to it.
pub(super) fn stored_details(grant: &OAuthClientGrant) -> Result<Option<ResolvedDetails>, ()> {
    grant
        .authorization_details
        .as_deref()
        .map(GrantDocument::from_document)
        .transpose()
        .map_err(
            |e| error!(grant_id = %grant.id, "A grant's authorization details do not read: {e}"),
        )?
        .map(|document| document.data_access().ok_or(()))
        .transpose()
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
    // An agent-write approval is never remembered: each one is a fresh
    // sign-in and the athlete's own decision (owl PDPP §4.5, §6.4).
    if checked.agent_write.is_some() {
        return false;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "https://api.dravr.test/mcp";

    fn subject() -> ConsentSubject<'static> {
        ConsentSubject {
            client_id: "https://client.example/meta.json",
            redirect_uri: "https://client.example/callback",
            first_party_only_sources: &[],
        }
    }

    /// A scope-only request the picker answers.
    fn scope_only() -> CheckedAuthorization {
        CheckedAuthorization {
            scope: "fitness:read".to_owned(),
            resource: None,
            client_name: None,
            consent_every_time: false,
            unstored_client: None,
            client_host_profile: None,
            admin_grantable: false,
            details: None,
            agent_write: None,
            preset_source: Some(SOURCE.to_owned()),
        }
    }

    /// The catalogue, with the standing-policy box worded in English.
    fn worded() -> MessagingStringsRegistry {
        let strings = MessagingStringsRegistry::new();
        strings.update(
            KEY_CONSENT_STANDING_POLICY,
            "en",
            "Always allow {0} this preset".to_owned(),
            "worded".to_owned(),
        );
        strings
    }

    /// The catalogue with the standing-policy box's line blanked in English,
    /// as a live overlay could leave it: the embedded catalogue words it.
    fn unworded() -> MessagingStringsRegistry {
        let strings = MessagingStringsRegistry::new();
        strings.update(
            KEY_CONSENT_STANDING_POLICY,
            "en",
            String::new(),
            "blank".to_owned(),
        );
        strings
    }

    fn form(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_picker_form_naming_the_step_up_preset_grants_the_plain_choice() {
        let strings = worded();
        let (details, field) = chosen(
            &form(&[("grant", Preset::OneOffQuestion.name())]),
            &scope_only(),
            &strings,
        );
        assert!(
            details.is_none(),
            "bug: a picker form granted the step-up preset: {details:?}"
        );
        assert_eq!(
            field,
            picker_digest_field(EVERYTHING),
            "bug: the plain choice's digest"
        );
        let (details, _) = chosen(
            &form(&[("grant", "training_coach")]),
            &scope_only(),
            &strings,
        );
        assert!(details.is_some(), "control: an offered preset is granted");
    }

    #[test]
    fn the_box_is_read_only_beside_the_chosen_preset_while_it_is_offered() {
        let strings = worded();
        let text = ConsentText::new(&strings, "en");
        let checked = scope_only();
        let ticked = form(&[
            ("grant", "training_coach"),
            ("standing_policy_training_coach", "on"),
        ]);
        assert!(
            standing_policy_ticked(&ticked, &checked, &text, true),
            "bug: a ticked box beside the chosen preset is not read"
        );
        assert!(
            !standing_policy_ticked(&ticked, &checked, &text, false),
            "bug: a box field is read while the box is not offered"
        );
        let plain = form(&[
            ("grant", "everything"),
            ("standing_policy_training_coach", "on"),
        ]);
        assert!(
            !standing_policy_ticked(&plain, &checked, &text, true),
            "bug: a box is read on the plain grant"
        );
        let other = form(&[
            ("grant", "sleep_coach"),
            ("standing_policy_training_coach", "on"),
        ]);
        assert!(
            !standing_policy_ticked(&other, &checked, &text, true),
            "bug: another preset's box is read for the chosen one"
        );
        let unworded = unworded();
        assert!(
            !standing_policy_ticked(&ticked, &checked, &ConsentText::new(&unworded, "en"), true),
            "bug: a box the catalogue does not word is read"
        );
    }

    #[test]
    fn the_box_renders_beside_each_preset_only_when_offered_and_worded() {
        let strings = worded();
        let text = ConsentText::new(&strings, "en");
        let checked = scope_only();
        let offered = GrantChoice::for_screen(&checked, None, &text, subject(), true).unwrap();
        let page = offered.render("<admin-box>", "Client", &text);
        for preset in ordinary_presets() {
            assert!(
                page.contains(&format!(r#"name="standing_policy_{}""#, preset.name())),
                "bug: no box beside {}: {page}",
                preset.name()
            );
        }
        assert!(
            page.contains("Always allow Client this preset"),
            "bug: the box's line is not shown: {page}"
        );
        assert!(
            !page.contains("one_off_question"),
            "bug: ordinary consent offered the step-up preset: {page}"
        );
        assert_eq!(
            page.matches("standing-policy").count(),
            ordinary_presets().count(),
            "bug: the plain choice carries a box: {page}"
        );

        let off = GrantChoice::for_screen(&checked, None, &text, subject(), false).unwrap();
        assert!(
            !off.render("", "Client", &text).contains("standing_policy"),
            "bug: the box shows on a screen that offers none"
        );
        let unworded = unworded();
        let unworded_text = ConsentText::new(&unworded, "en");
        let unlabelled =
            GrantChoice::for_screen(&checked, None, &unworded_text, subject(), true).unwrap();
        assert!(
            !unlabelled
                .render("", "Client", &unworded_text)
                .contains("standing_policy"),
            "bug: an unlabelled box is shown"
        );
    }

    #[test]
    fn every_stream_a_one_off_question_may_pick_has_its_label() {
        for stream in Preset::OneOffQuestion.streams() {
            assert!(
                step_up_stream_key(*stream).is_some(),
                "bug: {} has no step-up label",
                stream.name()
            );
        }
        assert!(
            step_up_stream_key(Stream::RestrictedNotes).is_none(),
            "bug: a T3 stream has a step-up label"
        );
    }
}
