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
//! as one precise number, and an estimate never as a measurement, whatever
//! its confidence. No sentence frames load as risk.
//!
//! The notice is stored with the reply as an `envelope_notice` entry, as the
//! plan card is, so a reload shows it again; the envelope renders it as its
//! own block where the surface has one and folds it into the prose otherwise.

use dravr_tronc::dates::civil_clock::format_civil_day;
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_contremaitre::messaging_strings::{
    envelope_purpose_key, MessagingStringsRegistry, KEY_ENVELOPE_DAY, KEY_ENVELOPE_HEADING,
    KEY_ENVELOPE_RELABELLED,
};
use pierre_core::models::{UnitSystem, ENVELOPE_NOTICE_BLOCK_TYPE};
use pierre_database::database::ConversationRecord;
use pierre_memory::training_plans;
use pierre_tool_runtime::envelope_notices::NoticeDay;
use pierre_tool_runtime::envelope_words::{anchor_sentence, filled, finding_sentence};
use serde_json::{json, Value};
use tracing::warn;

use super::plan_block::append_block;
use crate::turn_stop::settled_stop_marker;

/// Who reads a reply in `conv`, which decides how an anchor the athlete set
/// is named: a coach in a thread about one athlete, the athlete otherwise.
pub(crate) fn notice_reader(conv: &ConversationRecord) -> AnchorReader {
    if conv.subject().is_some() {
        AnchorReader::Coach
    } else {
        AnchorReader::Athlete
    }
}

/// The blocks of a reply the platform wrote in the model's place: a Guardian
/// block, a reconnect prompt, or a withheld reply (a stopped notice is the
/// fourth, in [`stop_notice_blocks`]). Every edit earns its
/// warning (D3) whatever answered the turn, so they carry the notice alone;
/// `None` when this turn's writes found nothing.
pub(crate) fn notice_only_blocks(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    units: UnitSystem,
    days: &[NoticeDay],
) -> Option<String> {
    envelope_notice_block(strings, locale, reader, units, days)
        .and_then(|block| append_block(None, block))
}

/// The two values a stopped notice carries the turn's envelope notice in.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct StopNoticeBlocks {
    /// The notice row's `content_blocks`: the [`settled_stop_marker`] naming the
    /// question it closed, then the notice. Built whole, never appended to
    /// what the row holds, so writing it twice stores it once.
    pub(crate) row: String,
    /// The envelope's: the notice alone, since the envelope's block walk would
    /// read the marker as a chart.
    pub(crate) envelope: String,
}

/// The blocks of a stopped notice, the other reply the platform writes in the
/// model's place (carnet#1519): a turn the athlete stopped after a write went
/// through still earns that write's warning (D3). `None` when this turn's
/// writes found nothing, which leaves the notice as the stop request wrote it.
pub(crate) fn stop_notice_blocks(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    units: UnitSystem,
    question_id: &str,
    days: &[NoticeDay],
) -> Option<StopNoticeBlocks> {
    let block = envelope_notice_block(strings, locale, reader, units, days)?;
    let row = append_block(settled_stop_marker(question_id), block.clone())?;
    let envelope = append_block(None, block)?;
    Some(StopNoticeBlocks { row, envelope })
}

/// The notice for this turn's `days`, as the stored entry the reply carries,
/// a speed written as pace in the reader's `units`; `None` when no day renders
/// a sentence, or the strings are not loaded.
pub(crate) fn envelope_notice_block(
    strings: &MessagingStringsRegistry,
    locale: &str,
    reader: AnchorReader,
    units: UnitSystem,
    days: &[NoticeDay],
) -> Option<Value> {
    let lines: Vec<String> = days
        .iter()
        .filter_map(|day| day_line(strings, locale, reader, units, day))
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
    reader: AnchorReader,
    units: UnitSystem,
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
        .and_then(|anchor| anchor_sentence(strings, locale, reader, units, anchor))
    {
        sentences.push(sentence);
    }
    let date = training_plans::parse_plan_date(&day.date)
        .map_or_else(|| day.date.clone(), |date| format_civil_day(date, locale));
    filled(strings.render(KEY_ENVELOPE_DAY, locale, &[&date, &sentences.join(" ")]))
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::slice;

    use super::*;
    use pierre_core::models::periodization::{
        AnchorConfidence, AnchorValue, EnvelopeFinding, SportFamily, WorkoutPurpose,
    };
    use pierre_core::models::{MeasurementKind, SportAnchorName, TargetAnchor};
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
            position: Some(0),
            findings,
            relabelled_purpose: None,
            anchor: None,
            source: NoticeSource::PlanDay,
        }
    }

    fn lines(block: &Value) -> Vec<String> {
        block["lines"].as_array().map_or_default(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_owned))
                .collect()
        })
    }

    #[test]
    fn a_relabelled_day_names_the_move_the_new_purpose_and_its_anchor() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut notice = day(vec![finding("cycle_mean_floor", Some(71.6), Some(80.0))?]);
        notice.relabelled_purpose = Some(WorkoutPurpose::Threshold);
        notice.anchor = Some(TargetAnchor {
            sport: Some(SportFamily::Ride),
            anchor: SportAnchorName::Ftp,
            kind: Some(MeasurementKind::Measured),
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
        let block = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[notice],
        )
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
            kind: Some(MeasurementKind::Measured),
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
        let block = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Coach,
            UnitSystem::Metric,
            &[notice],
        )
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
            kind: Some(MeasurementKind::Measured),
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
        let block = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[notice],
        )
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
    fn an_estimate_at_high_confidence_reads_as_an_estimate_never_a_measurement() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let anchored = |kind: MeasurementKind| {
            let mut notice = day(vec![finding("cycle_mean_floor", Some(71.6), Some(80.0))?]);
            notice.anchor = Some(TargetAnchor {
                sport: Some(SportFamily::Ride),
                anchor: SportAnchorName::Ftp,
                kind: Some(kind),
                value: AnchorValue {
                    value: 285.0,
                    low: Some(275.0),
                    high: Some(295.0),
                    origin: Some("intervals.icu".to_owned()),
                    as_of: None,
                    method: None,
                    confidence: AnchorConfidence::High,
                },
                own_conversions: false,
            });
            Ok::<_, Box<dyn Error>>(notice)
        };
        let estimated = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[anchored(MeasurementKind::Estimated)?],
        )
        .ok_or("a notice")?;
        let line = lines(&estimated).join("\n");
        assert!(
            line.contains("Anchor: FTP about 285 W (estimate), from intervals.icu."),
            "bug: an estimate stored at high confidence reads as a measured number: {line}"
        );

        let french = envelope_notice_block(
            &strings,
            "fr",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[anchored(MeasurementKind::Estimated)?],
        )
        .ok_or("a notice")?;
        let line = lines(&french).join("\n");
        assert!(
            line.contains("environ 285 W (estimation)"),
            "bug: the estimate marker is written in the reader's language: {line}"
        );

        let measured = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[anchored(MeasurementKind::Measured)?],
        )
        .ok_or("a notice")?;
        let line = lines(&measured).join("\n");
        assert!(
            line.contains("Anchor: FTP 285 W, from intervals.icu."),
            "bug: a measured high-confidence anchor reads as its value: {line}"
        );
        assert!(
            !line.contains("estimate"),
            "bug: a measurement is not marked as an estimate: {line}"
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
            AnchorReader::Athlete,
            UnitSystem::Metric,
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
            AnchorReader::Athlete,
            UnitSystem::Metric,
            slice::from_ref(&ratio),
        )
        .ok_or("a notice")?;
        assert!(
            lines(&fr).join("\n").contains("2,5:1"),
            "bug: French writes the ratio with a decimal comma"
        );
        let en = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            &[ratio],
        )
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
            envelope_notice_block(
                &strings,
                "en",
                AnchorReader::Athlete,
                UnitSystem::Metric,
                &[day(Vec::new())]
            )
            .is_none(),
            "bug: a day with no finding has nothing to say"
        );
        let missing = day(vec![finding("cycle_mean_floor", None, None)?]);
        assert!(
            envelope_notice_block(
                &strings,
                "en",
                AnchorReader::Athlete,
                UnitSystem::Metric,
                &[missing]
            )
            .is_none(),
            "bug: a sentence with an unfilled slot is withheld, never shown with {{0}}"
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
            AnchorReader::Athlete,
            UnitSystem::Metric,
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
            notice_only_blocks(
                &strings,
                "en",
                AnchorReader::Athlete,
                UnitSystem::Metric,
                &[day(Vec::new())]
            )
            .is_none(),
            "bug: a turn whose writes found nothing adds no block to a reply the platform wrote"
        );
        Ok(())
    }

    #[test]
    fn a_stopped_notice_keeps_its_marker_and_carries_the_notice_once() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let warned = day(vec![finding("cycle_mean_floor", Some(71.6), Some(80.0))?]);
        let blocks = stop_notice_blocks(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            "question-1",
            slice::from_ref(&warned),
        )
        .ok_or("a notice for a day with a finding")?;
        let row: Vec<Value> = serde_json::from_str(&blocks.row)?;
        assert_eq!(
            row.len(),
            2,
            "bug: the stopped row holds its marker and the notice, nothing else: {row:#?}"
        );
        let marker: Vec<Value> =
            serde_json::from_str(&settled_stop_marker("question-1").ok_or("a marker")?)?;
        assert_eq!(
            row[0], marker[0],
            "bug: the row keeps the marker naming its question, or the question reads unanswered"
        );
        assert_eq!(
            row[1]["type"], ENVELOPE_NOTICE_BLOCK_TYPE,
            "bug: the notice follows the marker on the stopped row: {row:#?}"
        );
        let envelope: Vec<Value> = serde_json::from_str(&blocks.envelope)?;
        assert_eq!(
            envelope,
            vec![row[1].clone()],
            "bug: the envelope carries the notice alone; its block walk reads a marker as a chart"
        );
        let again = stop_notice_blocks(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Metric,
            "question-1",
            &[warned],
        );
        assert_eq!(
            again.as_ref(),
            Some(&blocks),
            "bug: the watch and the turn both write the stopped row; the same findings must \
             write the same value, so the second write stores the notice once"
        );
        assert!(
            stop_notice_blocks(
                &strings,
                "en",
                AnchorReader::Athlete,
                UnitSystem::Metric,
                "question-1",
                &[day(Vec::new())]
            )
            .is_none(),
            "bug: a stopped turn whose writes found nothing leaves the notice as written"
        );
        Ok(())
    }

    #[test]
    fn an_imperial_readers_notice_writes_the_anchor_pace_per_mile() -> TestResult {
        let strings = MessagingStringsRegistry::new();
        let mut notice = day(vec![finding("map_without_definition", None, None)?]);
        notice.anchor = Some(TargetAnchor {
            sport: Some(SportFamily::Run),
            anchor: SportAnchorName::ThresholdSpeed,
            kind: Some(MeasurementKind::Measured),
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
        let imperial = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Imperial,
            slice::from_ref(&notice),
        )
        .ok_or("a notice")?;
        let line = lines(&imperial).join("\n");
        assert!(
            line.contains("threshold pace 6:42/mi"),
            "bug: the anchor's pace follows the reader's imperial setting: {line}"
        );
        assert!(
            !line.contains("/km"),
            "bug: an imperial reader sees no pace per km: {line}"
        );

        let mut estimate = day(vec![finding("map_without_definition", None, None)?]);
        estimate.anchor = Some(TargetAnchor {
            sport: Some(SportFamily::Run),
            anchor: SportAnchorName::CriticalSpeed,
            kind: Some(MeasurementKind::Estimated),
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
        let range = envelope_notice_block(
            &strings,
            "en",
            AnchorReader::Athlete,
            UnitSystem::Imperial,
            &[estimate],
        )
        .ok_or("a notice")?;
        let line = lines(&range).join("\n");
        assert!(
            line.contains("critical speed about 6:34–6:51/mi (estimate)"),
            "bug: both ends of a low-confidence range read per mile on imperial \
             (1609.344 m at 4.08 and 3.92 m/s): {line}"
        );
        Ok(())
    }
}
