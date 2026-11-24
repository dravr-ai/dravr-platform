// ABOUTME: The words of one envelope finding and of the anchor a session's targets sit on, from the fixed messaging.envelope.* sentences
// ABOUTME: One renderer for the envelope notice under a reply and the coach's family editor, so both say a finding the same way

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Envelope sentences (carnet#1495, carnet#1515).
//!
//! The chat pipeline's envelope notice writes each finding of the warn-only
//! envelope check, and the anchor a session's targets sit on, from the
//! dravr-contremaitre `messaging.envelope.*` strings. The coach's "build from
//! family" editor shows the same findings live, before the session is saved;
//! both read their sentences here, so the editor's line and the notice the
//! save would have earned are the same words. A low-confidence anchor is
//! never shown as one precise number, and an estimate never as a
//! measurement, whatever its confidence. No sentence frames load as risk.

use pierre_contremaitre::anchor_strings::{anchor_source_text, anchor_value_text, AnchorReader};
use pierre_contremaitre::messaging_strings::{
    envelope_anchor_label_key, envelope_finding_key, envelope_param_key, MessagingStringsRegistry,
    KEY_ENVELOPE_ANCHOR,
};
use pierre_core::models::periodization::{EnvelopeFinding, FindingCode, SportFamily};
use pierre_core::models::{TargetAnchor, UnitSystem};
use pierre_services::notification_text;
use tracing::warn;

/// The preset parameters a stated value can leave the range of, each with its
/// own sentence; any other key reads through the generic one.
const PARAM_KEYS: [&str; 6] = [
    "sets",
    "reps",
    "work_seconds",
    "rest_seconds",
    "set_rest_seconds",
    "duration_minutes",
];

/// The stated preset value a finding is about: its key when the finding is a
/// value outside its preset's range, `None` for every other finding.
#[must_use]
pub fn stated_param(finding: &EnvelopeFinding) -> Option<&'static str> {
    match (finding.code, finding.key.as_deref()) {
        (FindingCode::ParamOutOfRange, Some(param)) => {
            PARAM_KEYS.iter().copied().find(|known| *known == param)
        }
        _ => None,
    }
}

/// The sentence for one finding, its numbers filled; `None` when its string
/// is missing or a slot it names has no number.
#[must_use]
pub fn finding_sentence(
    strings: &MessagingStringsRegistry,
    locale: &str,
    finding: &EnvelopeFinding,
) -> Option<String> {
    let key = stated_param(finding).map_or_else(
        || envelope_finding_key(finding.code.as_str()),
        envelope_param_key,
    );
    let ratio = matches!(finding.code, FindingCode::RatioCeiling);
    // A missing number leaves its slot as written, so `filled` withholds the
    // sentence instead of showing it with an empty gap.
    let number = |value: Option<f32>, slot: usize| {
        value.map_or_else(
            || format!("{{{slot}}}"),
            |v| {
                if ratio {
                    notification_text::decimal(f64::from(v), locale)
                } else {
                    format!("{:.0}", v.round())
                }
            },
        )
    };
    let rendered = filled(strings.render(
        &key,
        locale,
        &[&number(finding.measured, 0), &number(finding.bound, 1)],
    ));
    if rendered.is_none() {
        warn!(
            code = finding.code.as_str(),
            locale, "envelope sentence: a finding has no string or no number for its slot"
        );
    }
    rendered
}

/// The anchor sentence: its label, its value (a range or an estimate at low
/// confidence, an estimate at any confidence unless measured, a speed as pace
/// in `units`), and who set it, named for `reader`.
#[must_use]
pub fn anchor_sentence(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    units: UnitSystem,
    anchor: &TargetAnchor,
) -> Option<String> {
    let label = strings.get(&envelope_anchor_label_key(anchor.anchor.as_str()), locale);
    if label.is_empty() {
        return None;
    }
    // An anchor stored before it named its sport: a speed reads as run pace,
    // the sport the profile row's speeds belong to.
    let sport = anchor.sport.unwrap_or(SportFamily::Run);
    let value = anchor_value_text(
        strings,
        locale,
        anchor.anchor.unit(),
        sport,
        units,
        &anchor.value,
        anchor.kind,
    );
    let source = anchor_source_text(strings, locale, reader, anchor.value.origin.as_deref());
    filled(strings.render(KEY_ENVELOPE_ANCHOR, locale, &[&label, &value, &source]))
}

/// `rendered` when it carries text and no slot left unfilled.
#[must_use]
pub fn filled(rendered: String) -> Option<String> {
    let complete = !rendered.trim().is_empty()
        && !(0..4).any(|slot| rendered.contains(&format!("{{{slot}}}")));
    complete.then_some(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_with_a_slot_left_open_is_withheld() {
        assert_eq!(
            filled("Work bouts of 40 s are outside the range (limit {1} s).".to_owned()),
            None,
            "bug: a sentence missing a number is shown with a gap"
        );
        assert_eq!(
            filled("   ".to_owned()),
            None,
            "bug: an empty sentence is shown"
        );
        assert_eq!(
            filled("Anchor: FTP 280 W, your own.".to_owned()).as_deref(),
            Some("Anchor: FTP 280 W, your own."),
            "bug: a complete sentence is withheld"
        );
    }
}
