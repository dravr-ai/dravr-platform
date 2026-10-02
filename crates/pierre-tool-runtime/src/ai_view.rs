// ABOUTME: The tool output boundary of the AI policies — filters each tool's structured result and notes what was withheld
// ABOUTME: Runs in the executor after every tool body, which itself runs as a read for a model (ai_scope::ai_read)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What leaves a tool for a model (carnet#723).
//!
//! The executor runs every tool body inside
//! [`ai_read`](pierre_providers::ai_scope::ai_read), so the provider policies
//! already govern what the tool read — every provider it obtained is an
//! [`AiGovernedProvider`](pierre_providers::ai_scope::AiGovernedProvider).
//! [`withhold_from_model`] is the backstop on the way out: it filters any
//! provider item still in the structured result, and adds one neutral note
//! when anything was held back, so the model says data is unavailable instead
//! of inventing it.

use pierre_core::ai_policy::{filter_json, AiPolicyLookup, Withheld};
use pierre_providers::ai_scope;
use serde_json::Value;

use crate::protocol::types::UniversalResponse;

/// Key of the note added to a tool result that had data withheld.
pub const WITHHELD_KEY: &str = "_withheld";

/// Filter every provider item left in a tool's structured result, and add one
/// note when anything — here or in the tool's reads (`read_side`) — was held
/// back. Nothing happens for a call made for display.
pub(crate) fn withhold_from_model(
    lookup: &dyn AiPolicyLookup,
    response: &mut UniversalResponse,
    read_side: Withheld,
) {
    if ai_scope::lifted() {
        return;
    }
    let mut withheld = read_side;
    if let Some(result) = response.result.as_mut() {
        withheld.merge(filter_json(lookup, result));
    }
    if withheld.is_empty() {
        return;
    }
    // The note rides in the result: the chat loop and MCP clients read the
    // result, never `metadata`. A result that is not an object is wrapped,
    // which only ever happens to a result that had something withheld.
    match response.result.take() {
        Some(Value::Object(mut object)) => {
            object.insert(WITHHELD_KEY.to_owned(), withheld.to_note());
            response.result = Some(Value::Object(object));
        }
        other => {
            response.result = Some(serde_json::json!({
                "result": other.unwrap_or(Value::Null),
                WITHHELD_KEY: withheld.to_note(),
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_providers::ai_scope::{ai_read, filter_activities, for_display};
    use pierre_providers::provider_ai_terms::{NOLIO, WHOOP};
    use serde_json::json;

    struct Lookup;

    impl AiPolicyLookup for Lookup {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "nolio" => Some(&NOLIO),
                "whoop" => Some(&WHOOP),
                _ => None,
            }
        }
    }

    fn activity(id: &str, source: Option<&str>) -> Activity {
        let start = Utc.with_ymd_and_hms(2026, 9, 30, 6, 0, 0).unwrap();
        ActivityBuilder::new(id, "Hills", SportType::Ride, start, 3600, "nolio")
            .average_heart_rate(150)
            .source_opt(source.map(str::to_owned))
            .build()
    }

    fn response(result: Value) -> UniversalResponse {
        UniversalResponse {
            success: true,
            result: Some(result),
            error: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn read_side_filtering_reaches_the_tool_result_note() {
        let ((), read_side) = ai_read(async {
            let kept = filter_activities(
                &Lookup,
                vec![
                    activity("g", Some("garmin")),
                    activity("s", Some("strava")),
                    activity("z", Some("zepp")),
                ],
            );
            assert_eq!(kept.len(), 2);
            assert_eq!(kept[0].average_heart_rate(), Some(150));
            assert_eq!(kept[1].average_heart_rate(), None, "strava: existence only");
            assert_eq!(kept[1].duration_seconds(), 3600);
        })
        .await;

        let mut response = response(json!({ "activity_list": "1 ride" }));
        withhold_from_model(&Lookup, &mut response, read_side);
        let note = &response.result.as_ref().unwrap()[WITHHELD_KEY];
        assert_eq!(note["items_dropped"], 1);
        assert_eq!(note["items_reduced"], 1);
        assert_eq!(note["sources"], json!(["strava", "zepp"]));
    }

    #[tokio::test]
    async fn outside_an_ai_read_nothing_is_filtered() {
        let kept = filter_activities(&Lookup, vec![activity("z", Some("zepp"))]);
        assert_eq!(kept.len(), 1, "the athlete's own surfaces see everything");
    }

    #[tokio::test]
    async fn display_runs_unfiltered_and_unannotated() {
        let (kept, withheld) = for_display(ai_read(async {
            filter_activities(&Lookup, vec![activity("z", Some("zepp"))])
        }))
        .await;
        assert_eq!(kept.len(), 1);

        let mut payload = response(json!([{ "provider": "nolio", "source": "zepp" }]));
        for_display(async { withhold_from_model(&Lookup, &mut payload, withheld) }).await;
        assert_eq!(
            payload.result,
            Some(json!([{ "provider": "nolio", "source": "zepp" }]))
        );
    }

    #[test]
    fn an_array_result_is_wrapped_so_the_model_reads_its_note() {
        let mut payload = response(json!([
            { "provider": "whoop", "recovery_score": 40.0, "hrv_ms": 60.0 }
        ]));
        withhold_from_model(&Lookup, &mut payload, Withheld::default());
        let result = payload.result.expect("a result");
        assert_eq!(
            result["result"],
            json!([{ "provider": "whoop", "hrv_ms": 60.0 }])
        );
        assert_eq!(result[WITHHELD_KEY]["items_reduced"], 1);
        assert!(payload.metadata.is_none());
    }

    #[test]
    fn nothing_restricted_leaves_the_result_untouched() {
        let original = json!({ "activities": [{ "provider": "strava", "name": "Tempo" }] });
        let mut payload = response(original.clone());
        withhold_from_model(&Lookup, &mut payload, Withheld::default());
        assert_eq!(payload.result, Some(original));
        assert!(payload.metadata.is_none());
    }
}
