// ABOUTME: Stage 15.8 — the envelope notice: what the warn-only envelope check found on this turn's sessions
// ABOUTME: Written by the platform from fixed translated sentences, so the model can explain a warning but never drop it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The envelope notice, produced (carnet#1495, plan step 12).
//!
//! Every edit to a workout family's session goes through (founder decision
//! D3); one outside the family's evidence envelope earns a warning, and one
//! that changes the physiology relabels the session. The tool results carry
//! the findings as data, and until now the athlete heard of them only if the
//! model relayed them. This stage writes them itself, under the reply, from
//! the dravr-contremaitre `messaging.envelope.*` sentences: a heading, then
//! one line per session naming the day, each finding's domain move with its
//! numbers, the purpose a relabel files it under, and the anchor its targets
//! sit on with that anchor's source. A low-confidence anchor is never shown
//! as one precise number. No sentence frames load as risk.
//!
//! The notice is stored with the reply as an `envelope_notice` entry, as the
//! plan card is, so a reload shows it again; the envelope renders it as its
//! own block where the surface has one and folds it into the prose otherwise.

use pierre_contremaitre::messaging_strings::{
    envelope_anchor_label_key, envelope_finding_key, envelope_param_key, envelope_purpose_key,
    MessagingStringsRegistry, KEY_ENVELOPE_ANCHOR, KEY_ENVELOPE_ANCHOR_ESTIMATE,
    KEY_ENVELOPE_ANCHOR_RANGE, KEY_ENVELOPE_DAY, KEY_ENVELOPE_HEADING, KEY_ENVELOPE_RELABELLED,
    KEY_ENVELOPE_SOURCE_ATHLETE, KEY_ENVELOPE_SOURCE_FROM, KEY_ENVELOPE_SOURCE_OWN,
};
use pierre_core::civil_time::format_civil_day;
use pierre_core::models::periodization::{
    AnchorConfidence, EnvelopeFinding, FindingCode, SportFamily,
};
use pierre_core::models::{
    AnchorOrigin, AnchorUnit, TargetAnchor, ATHLETE_REPORTED_ORIGIN, ENVELOPE_NOTICE_BLOCK_TYPE,
};
use pierre_database::database::ConversationRecord;
use pierre_memory::training_plans;
use pierre_services::notification_text;
use pierre_tool_runtime::envelope_notices::NoticeDay;
use serde_json::{json, Value};
use tracing::warn;

use super::plan_block::append_block;

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

/// Seconds in a minute, for writing a pace.
const SECONDS_PER_MINUTE: u32 = 60;

/// The distance a run pace is written per, in metres.
const RUN_PACE_METRES: f64 = 1_000.0;

/// The distance a swim pace is written per, in metres.
const SWIM_PACE_METRES: f64 = 100.0;

/// Who reads the notice, which decides how an anchor the athlete set is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoticeReader {
    /// The athlete, in their own thread: "your own value".
    Athlete,
    /// A coach, in a thread about one athlete: "the athlete's own value".
    Coach,
}

impl NoticeReader {
    /// Who reads a reply in `conv`: a coach in a thread about one athlete,
    /// the athlete otherwise.
    pub(crate) fn of(conv: &ConversationRecord) -> Self {
        if conv.subject().is_some() {
            Self::Coach
        } else {
            Self::Athlete
        }
    }
}

/// The blocks of a reply the platform wrote in the model's place: a Guardian
/// block, a reconnect prompt, or a withheld reply. Every edit earns its
/// warning (D3) whatever answered the turn, so they carry the notice alone;
/// `None` when this turn's writes found nothing.
pub(crate) fn notice_only_blocks(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: NoticeReader,
    days: &[NoticeDay],
) -> Option<String> {
    envelope_notice_block(strings, locale, reader, days).and_then(|block| append_block(None, block))
}

/// The notice for this turn's `days`, as the stored entry the reply carries;
/// `None` when no day renders a sentence, or the strings are not loaded.
pub(crate) fn envelope_notice_block(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: NoticeReader,
    days: &[NoticeDay],
) -> Option<Value> {
    let lines: Vec<String> = days
        .iter()
        .filter_map(|day| day_line(strings, locale, reader, day))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let heading = strings.get(KEY_ENVELOPE_HEADING, locale);
    if heading.is_empty() {
        warn!(
            locale,
            "envelope notice: no heading string; the notice is not shown"
        );
        return None;
    }
    Some(json!({
        "type": ENVELOPE_NOTICE_BLOCK_TYPE,
        "heading": heading,
        "lines": lines,
    }))
}

/// One session's line: its day, then every sentence about it.
fn day_line(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: NoticeReader,
    day: &NoticeDay,
) -> Option<String> {
    let mut sentences: Vec<String> = day
        .findings
        .iter()
        .filter_map(|finding| finding_sentence(strings, locale, finding))
        .collect();
    if sentences.is_empty() {
        return None;
    }
    if let Some(purpose) = day.relabelled_purpose {
        let label = strings.get(&envelope_purpose_key(purpose.as_str()), locale);
        if let Some(sentence) = filled(strings.render(KEY_ENVELOPE_RELABELLED, locale, &[&label]))
            .filter(|_| !label.is_empty())
        {
            sentences.push(sentence);
        }
    }
    if let Some(sentence) = day
        .anchor
        .as_ref()
        .and_then(|anchor| anchor_sentence(strings, locale, reader, anchor))
    {
        sentences.push(sentence);
    }
    let date = training_plans::parse_plan_date(&day.date)
        .map_or_else(|| day.date.clone(), |date| format_civil_day(date, locale));
    filled(strings.render(KEY_ENVELOPE_DAY, locale, &[&date, &sentences.join(" ")]))
}

/// The sentence for one finding, its numbers filled; `None` when its string
/// is missing or a slot it names has no number.
fn finding_sentence(
    strings: &MessagingStringsRegistry,
    locale: &str,
    finding: &EnvelopeFinding,
) -> Option<String> {
    let key = match (finding.code, finding.key.as_deref()) {
        (FindingCode::ParamOutOfRange, Some(param)) if PARAM_KEYS.contains(&param) => {
            envelope_param_key(param)
        }
        (code, _) => envelope_finding_key(code.as_str()),
    };
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
            locale, "envelope notice: a finding has no string or no number for its slot"
        );
    }
    rendered
}

/// The anchor sentence: its label, its value (a range or an estimate at low
/// confidence), and who set it.
fn anchor_sentence(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: NoticeReader,
    anchor: &TargetAnchor,
) -> Option<String> {
    let label = strings.get(&envelope_anchor_label_key(anchor.anchor.as_str()), locale);
    if label.is_empty() {
        return None;
    }
    let unit = anchor.anchor.unit();
    // An anchor stored before it named its sport: a speed reads as run pace,
    // the sport the profile row's speeds belong to.
    let sport = anchor.sport.unwrap_or(SportFamily::Run);
    let point = &anchor.value;
    let value = if point.confidence == AnchorConfidence::Low {
        match (point.low, point.high) {
            (Some(low), Some(high)) => {
                let (first, second) = ordered_for_display(unit, low, high);
                strings.render(
                    KEY_ENVELOPE_ANCHOR_RANGE,
                    locale,
                    &[&bare(unit, sport, first), &with_unit(unit, sport, second)],
                )
            }
            _ => strings.render(
                KEY_ENVELOPE_ANCHOR_ESTIMATE,
                locale,
                &[&with_unit(unit, sport, point.value)],
            ),
        }
    } else {
        with_unit(unit, sport, point.value)
    };
    let origin = point
        .origin
        .as_deref()
        .unwrap_or(AnchorOrigin::Athlete.as_str());
    let source = if origin == AnchorOrigin::Athlete.as_str() || origin == ATHLETE_REPORTED_ORIGIN {
        strings.get(
            match reader {
                NoticeReader::Athlete => KEY_ENVELOPE_SOURCE_OWN,
                NoticeReader::Coach => KEY_ENVELOPE_SOURCE_ATHLETE,
            },
            locale,
        )
    } else {
        strings.render(KEY_ENVELOPE_SOURCE_FROM, locale, &[origin])
    };
    filled(strings.render(KEY_ENVELOPE_ANCHOR, locale, &[&label, &value, &source]))
}

/// A range's ends in the order they read: a pace range runs from the faster
/// pace (the higher speed) to the slower one.
fn ordered_for_display(unit: AnchorUnit, low: f32, high: f32) -> (f32, f32) {
    if unit == AnchorUnit::MetresPerSecond {
        (high, low)
    } else {
        (low, high)
    }
}

/// A value with its unit: watts, beats per minute, or a speed as a pace.
fn with_unit(unit: AnchorUnit, sport: SportFamily, value: f32) -> String {
    match unit {
        AnchorUnit::Watts => format!("{} W", bare(unit, sport, value)),
        AnchorUnit::BeatsPerMinute => format!("{} bpm", bare(unit, sport, value)),
        AnchorUnit::MetresPerSecond if sport == SportFamily::Swim => {
            format!("{}/100 m", bare(unit, sport, value))
        }
        AnchorUnit::MetresPerSecond => format!("{}/km", bare(unit, sport, value)),
    }
}

/// A value without its unit: a whole number, or a speed as `m:ss` of pace.
fn bare(unit: AnchorUnit, sport: SportFamily, value: f32) -> String {
    if unit != AnchorUnit::MetresPerSecond || value <= 0.0 {
        return format!("{:.0}", value.round());
    }
    let per = if sport == SportFamily::Swim {
        SWIM_PACE_METRES
    } else {
        RUN_PACE_METRES
    };
    // A pace of at most a few minutes: far inside u32.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let seconds = (per / f64::from(value)).round() as u32;
    format!(
        "{}:{:02}",
        seconds / SECONDS_PER_MINUTE,
        seconds % SECONDS_PER_MINUTE
    )
}

/// `rendered` when it carries text and no slot left unfilled.
fn filled(rendered: String) -> Option<String> {
    let complete = !rendered.trim().is_empty()
        && !(0..4).any(|slot| rendered.contains(&format!("{{{slot}}}")));
    complete.then_some(rendered)
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::slice;

    use super::*;
    use pierre_core::models::periodization::{AnchorValue, WorkoutPurpose};
    use pierre_core::models::SportAnchorName;
    use pierre_tool_runtime::envelope_notices::NoticeSource;
    use serde_json::json;

    type TestResult = Result<(), Box<dyn Error>>;

    fn finding(
        code: &str,
        measured: Option<f32>,
        bound: Option<f32>,
    ) -> Result<EnvelopeFinding, Box<dyn Error>> {
        Ok(serde_json::from_value(json!({
            "code": code, "action": "warn", "message": "English, for the model.",
            "measured": measured, "bound": bound
        }))?)
    }

    fn day(findings: Vec<EnvelopeFinding>) -> NoticeDay {
        NoticeDay {
            date: "2026-07-14".to_owned(),
            findings,
            relabelled_purpose: None,
            anchor: None,
            source: NoticeSource::PlanDay,
        }
    }

    fn lines(block: &Value) -> Vec<String> {
        block["lines"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(|line| line.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn a_relabelled_day_names_the_move_the_new_purpose_and_its_anchor() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut notice = day(vec![finding("cycle_mean_floor", Some(71.6), Some(80.0))?]);
        notice.relabelled_purpose = Some(WorkoutPurpose::Threshold);
        notice.anchor = Some(TargetAnchor {
            sport: Some(SportFamily::Ride),
            anchor: SportAnchorName::Ftp,
            value: AnchorValue {
                value: 285.0,
                low: None,
                high: None,
                origin: Some("intervals.icu".to_owned()),
                as_of: None,
                method: None,
                confidence: AnchorConfidence::Medium,
            },
            own_conversions: false,
        });
        let block = envelope_notice_block(&strings, "en", NoticeReader::Athlete, &[notice])
            .ok_or("a notice")?;
        assert_eq!(
            block["type"], ENVELOPE_NOTICE_BLOCK_TYPE,
            "bug: the stored entry must carry the envelope-notice discriminator"
        );
        assert!(
            block["heading"].as_str().is_some_and(|h| !h.is_empty()),
            "bug: a notice is never shown without its heading"
        );
        let line = lines(&block).join("\n");
        assert!(
            line.starts_with("2026-07-14"),
            "bug: the line opens on the session's day: {line}"
        );
        assert!(
            line.contains("about 72%"),
            "the measured cycle mean, rounded: {line}"
        );
        assert!(line.contains("below 80%"), "the bound: {line}");
        assert!(line.contains("threshold work"), "the relabel: {line}");
        assert!(
            line.contains("FTP 285 W, from intervals.icu"),
            "the anchor and source: {line}"
        );
        assert!(
            !line.to_lowercase().contains("risk"),
            "never framed as risk: {line}"
        );
        Ok(())
    }

    #[test]
    fn a_coach_reads_the_athletes_anchor_as_the_athletes() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut notice = day(vec![finding("map_without_definition", None, None)?]);
        notice.anchor = Some(TargetAnchor {
            sport: None,
            anchor: SportAnchorName::ThresholdSpeed,
            value: AnchorValue {
                value: 4.0,
                low: None,
                high: None,
                origin: Some("athlete".to_owned()),
                as_of: None,
                method: None,
                confidence: AnchorConfidence::High,
            },
            own_conversions: false,
        });
        let block = envelope_notice_block(&strings, "en", NoticeReader::Coach, &[notice])
            .ok_or("a notice")?;
        let line = lines(&block).join("\n");
        assert!(
            line.contains("the athlete's own value"),
            "bug: a coach's thread names the athlete's anchor as the reader's: {line}"
        );
        assert!(
            line.contains("threshold pace 4:10/km"),
            "bug: an anchor stored before it named its sport reads as run pace: {line}"
        );
        Ok(())
    }

    #[test]
    fn a_low_confidence_anchor_is_shown_as_a_range_never_a_point() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut notice = day(vec![finding("map_without_definition", None, None)?]);
        notice.anchor = Some(TargetAnchor {
            sport: Some(SportFamily::Run),
            anchor: SportAnchorName::CriticalSpeed,
            value: AnchorValue {
                value: 4.0,
                low: Some(3.92),
                high: Some(4.08),
                origin: Some("athlete".to_owned()),
                as_of: None,
                method: None,
                confidence: AnchorConfidence::Low,
            },
            own_conversions: false,
        });
        let block = envelope_notice_block(&strings, "en", NoticeReader::Athlete, &[notice])
            .ok_or("a notice")?;
        let line = lines(&block).join("\n");
        assert!(
            line.contains("critical speed about 4:05–4:15/km (estimate)"),
            "bug: a low-confidence pace reads as a range, faster end first: {line}"
        );
        assert!(
            line.contains("your own value"),
            "bug: an anchor the athlete set is named as theirs: {line}"
        );
        Ok(())
    }

    #[test]
    fn a_stated_value_outside_its_range_names_the_parameter_in_french() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut out_of_range = finding("param_out_of_range", Some(12.0), Some(10.0))?;
        out_of_range.key = Some("reps".to_owned());
        let block = envelope_notice_block(
            &strings,
            "fr",
            NoticeReader::Athlete,
            &[day(vec![out_of_range])],
        )
        .ok_or("a notice")?;
        let line = lines(&block).join("\n");
        assert!(
            line.contains("12 répétitions par série"),
            "bug: the parameter's own sentence names the stated value: {line}"
        );
        assert!(
            line.contains("limite 10"),
            "bug: the sentence names the bound: {line}"
        );
        assert!(
            line.contains(" : "),
            "French punctuation before the sentences: {line}"
        );
        Ok(())
    }

    #[test]
    fn a_ratio_keeps_one_decimal_in_the_locales_separator() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let ratio = day(vec![finding("ratio_ceiling", Some(2.5), Some(2.0))?]);
        let fr = envelope_notice_block(
            &strings,
            "fr",
            NoticeReader::Athlete,
            slice::from_ref(&ratio),
        )
        .ok_or("a notice")?;
        assert!(
            lines(&fr).join("\n").contains("2,5:1"),
            "bug: French writes the ratio with a decimal comma"
        );
        let en = envelope_notice_block(&strings, "en", NoticeReader::Athlete, &[ratio])
            .ok_or("a notice")?;
        assert!(
            lines(&en).join("\n").contains("2.5:1"),
            "bug: English writes the ratio with a decimal point"
        );
        Ok(())
    }

    #[test]
    fn a_day_with_no_finding_and_a_slot_with_no_number_say_nothing() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        assert!(
            envelope_notice_block(&strings, "en", NoticeReader::Athlete, &[day(Vec::new())])
                .is_none(),
            "bug: a day with no finding has nothing to say"
        );
        let missing = day(vec![finding("cycle_mean_floor", None, None)?]);
        assert!(
            envelope_notice_block(&strings, "en", NoticeReader::Athlete, &[missing]).is_none(),
            "a sentence with an unfilled slot is withheld, never shown with {{0}}"
        );
        Ok(())
    }

    #[test]
    fn a_reply_written_in_the_models_place_carries_the_notice_alone() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let warned = day(vec![finding("cycle_mean_floor", Some(71.6), Some(80.0))?]);
        let blocks = notice_only_blocks(
            &strings,
            "en",
            NoticeReader::Athlete,
            slice::from_ref(&warned),
        )
        .ok_or("a notice for a day with a finding")?;
        let stored: Vec<Value> = serde_json::from_str(&blocks)?;
        assert_eq!(
            stored.len(),
            1,
            "bug: a Guardian block, a reconnect prompt or a withheld reply carries the notice alone"
        );
        assert_eq!(
            stored[0]["type"], ENVELOPE_NOTICE_BLOCK_TYPE,
            "bug: stored as the entry Stage 15.8 stores, so a reload and every surface read it"
        );
        assert!(
            notice_only_blocks(&strings, "en", NoticeReader::Athlete, &[day(Vec::new())]).is_none(),
            "bug: a turn whose writes found nothing adds no block to a reply the platform wrote"
        );
        Ok(())
    }

    #[test]
    fn a_swim_speed_reads_as_pace_per_100_metres() {
        assert_eq!(
            with_unit(AnchorUnit::MetresPerSecond, SportFamily::Swim, 1.25),
            "1:20/100 m",
            "bug: a pool speed reads as pace per 100 m"
        );
        assert_eq!(
            with_unit(AnchorUnit::MetresPerSecond, SportFamily::Run, 4.0),
            "4:10/km",
            "bug: a run speed reads as pace per km"
        );
        assert_eq!(
            with_unit(AnchorUnit::BeatsPerMinute, SportFamily::Run, 171.6),
            "172 bpm",
            "bug: a heart rate reads as whole beats per minute"
        );
    }
}
