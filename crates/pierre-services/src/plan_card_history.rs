// ABOUTME: A stored workout_plan block's outside-the-evidence mark, worded on read the way a live card words it
// ABOUTME: Snapshots saved from 079ffba87 until carnet#1552 stored the mark as a bool; history serves them as the label
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan cards read back from history
//!
//! A reply that saved or read a plan stores its `workout_plan` block in the
//! message's `content_blocks`, and history serves that snapshot as it was
//! written. From 079ffba87 (carnet#1495, 2026-11-12) until the carnet#1552
//! change, a day's `outside_evidence` was stored as a bool, and every client
//! drew the warning's text from its own copy. Since carnet#1552 the card
//! carries the warning itself, worded by the server in the saving turn's
//! language ([`DayCard::outside_evidence`](crate::plan_card::DayCard)); the
//! clients word it in their reader's language and print that label only when
//! they hold no words for it (carnet#1813).
//!
//! The snapshots of that range stay as stored, so a rollback still reads
//! them; [`word_stored_outside_evidence`] rewrites them on the way out
//! instead: `true` becomes the label, `false` goes away, and a day that
//! already carries the label is left alone.

use pierre_contremaitre::evidence_strings;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use serde_json::{Map, Value};

use crate::plan_card::WORKOUT_PLAN_BLOCK_TYPE;

/// The field a plan card's day marks the warning with.
const OUTSIDE_EVIDENCE: &str = "outside_evidence";

/// Word the outside-the-evidence mark of every stored `workout_plan` block in
/// `blocks` in `locale`, as a live card words it.
///
/// For snapshots saved from 079ffba87 (carnet#1495) until the carnet#1552
/// change, whose days store the mark as a bool: `true` becomes the label
/// [`evidence_strings::outside_evidence_label`] resolves, or goes away when
/// that string is missing, as on a live card; `false` goes away, as a live
/// card leaves it absent. A day that carries the label, or no mark, and every
/// other block, are left as they are.
pub fn word_stored_outside_evidence(
    blocks: &mut [Value],
    registry: &MessagingStringsRegistry,
    locale: &str,
) {
    let mut label: Option<Option<String>> = None;
    let days = blocks
        .iter_mut()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some(WORKOUT_PLAN_BLOCK_TYPE))
        .filter_map(|block| block.pointer_mut("/plan/weeks")?.as_array_mut())
        .flatten()
        .filter_map(|week| week.get_mut("days")?.as_array_mut())
        .flatten()
        .filter_map(Value::as_object_mut);
    for day in days {
        let Some(&Value::Bool(outside)) = day.get(OUTSIDE_EVIDENCE) else {
            continue;
        };
        let worded = if outside {
            label
                .get_or_insert_with(|| evidence_strings::outside_evidence_label(registry, locale))
                .clone()
        } else {
            None
        };
        set_or_remove(day, worded);
    }
}

/// Put the worded mark on `day`, or take the field off when there is none.
fn set_or_remove(day: &mut Map<String, Value>, worded: Option<String>) {
    match worded {
        Some(label) => {
            day.insert(OUTSIDE_EVIDENCE.to_owned(), Value::String(label));
        }
        None => {
            day.remove(OUTSIDE_EVIDENCE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_contremaitre::evidence_strings::KEY_EVIDENCE_OUTSIDE;
    use serde_json::json;

    /// A stored block of two weeks, the marks of each day as given.
    fn stored_plan(first: &[Value], second: &[Value]) -> Value {
        let days = |marks: &[Value]| -> Vec<Value> {
            marks
                .iter()
                .map(|mark| {
                    let mut day = json!({ "date": "2026-11-16", "sport": "run", "rest": false });
                    if !mark.is_null() {
                        day[OUTSIDE_EVIDENCE] = mark.clone();
                    }
                    day
                })
                .collect()
        };
        json!({
            "type": WORKOUT_PLAN_BLOCK_TYPE,
            "source_tool": "save_training_plan",
            "plan": {
                "weeks": [
                    { "week_start": "2026-11-16", "days": days(first) },
                    { "week_start": "2026-11-23", "days": days(second) }
                ]
            }
        })
    }

    fn mark(block: &Value, week: usize, day: usize) -> Option<&Value> {
        block["plan"]["weeks"][week]["days"][day].get(OUTSIDE_EVIDENCE)
    }

    #[test]
    fn a_stored_true_reads_as_the_label_in_the_readers_language() {
        let strings = MessagingStringsRegistry::new();
        let mut blocks = vec![stored_plan(&[json!(true)], &[Value::Null, json!(true)])];

        word_stored_outside_evidence(&mut blocks, &strings, "fr");

        assert_eq!(
            mark(&blocks[0], 0, 0),
            Some(&json!("Hors des données probantes")),
            "bug: a stored mark reads as the warning in the reader's words"
        );
        assert_eq!(
            mark(&blocks[0], 1, 1),
            Some(&json!("Hors des données probantes")),
            "bug: every week's days are worded, not only the first's"
        );
        assert_eq!(
            mark(&blocks[0], 1, 0),
            None,
            "bug: a day with no mark gains none"
        );
    }

    #[test]
    fn a_stored_false_goes_away() {
        let strings = MessagingStringsRegistry::new();
        let mut blocks = vec![stored_plan(&[json!(false)], &[])];

        word_stored_outside_evidence(&mut blocks, &strings, "en");

        assert_eq!(
            mark(&blocks[0], 0, 0),
            None,
            "bug: an unmarked day carries no key, as on a live card"
        );
    }

    #[test]
    fn a_worded_mark_is_left_as_stored() {
        let strings = MessagingStringsRegistry::new();
        let mut blocks = vec![stored_plan(&[json!("Outside the evidence")], &[])];

        word_stored_outside_evidence(&mut blocks, &strings, "fr");

        assert_eq!(
            mark(&blocks[0], 0, 0),
            Some(&json!("Outside the evidence")),
            "bug: a card that already carries its label is served as written"
        );
    }

    #[test]
    fn a_stored_true_with_no_string_goes_away_never_empty() {
        let strings = MessagingStringsRegistry::new();
        strings.update(KEY_EVIDENCE_OUTSIDE, "en", String::new(), String::new());
        let mut blocks = vec![stored_plan(&[json!(true)], &[])];

        word_stored_outside_evidence(&mut blocks, &strings, "en");

        assert_eq!(
            mark(&blocks[0], 0, 0),
            None,
            "bug: a missing warning string never reaches a client as an empty label"
        );
    }

    #[test]
    fn every_other_block_is_left_untouched() {
        let strings = MessagingStringsRegistry::new();
        let chart = json!({ "type": "line_chart", "plan": { "weeks": [{ "days": [{ "outside_evidence": true }] }] } });
        let actions = json!({ "type": "actions", "actions": [] });
        let mut blocks = vec![chart.clone(), actions.clone()];

        word_stored_outside_evidence(&mut blocks, &strings, "en");

        assert_eq!(
            blocks,
            vec![chart, actions],
            "bug: only a workout_plan block is reworded"
        );
    }
}
