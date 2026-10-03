// ABOUTME: The tool output boundary of the provider terms — filters each tool's structured result and notes what was held back
// ABOUTME: Runs in the executor after every tool body, for a model (AI rules) and for an external caller (transport gate)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! What leaves a tool for its caller (carnet#723, carnet#724).
//!
//! The executor runs every tool body inside
//! [`ai_read`](pierre_providers::ai_scope::ai_read) and the transport its
//! entry point declared, so the provider terms already govern what the tool
//! read — every provider it obtained is an
//! [`AiGovernedProvider`](pierre_providers::ai_scope::AiGovernedProvider).
//! [`withhold_from_caller`] is the backstop on the way out: it filters any
//! provider item still in the structured result, and adds a neutral note for
//! each kind of hold-back, so the caller says data is unavailable instead of
//! inventing it.

use pierre_core::ai_policy::{filter_json, Exposure, ProviderTerms, Withheld};
use serde_json::{Map, Value};

use crate::protocol::types::UniversalResponse;

/// Key of the note added to a tool result that had data withheld from AI.
pub const WITHHELD_KEY: &str = "_withheld";

/// Key of the note added to a tool result that had data dropped because the
/// call came over a transport the data's terms keep it off.
pub const UNAVAILABLE_HERE_KEY: &str = "_unavailable_over_this_interface";

/// Filter every provider item left in a tool's structured result for the
/// gates `exposure` names, and add a note for each kind of hold-back — here or
/// in the tool's reads (`read_side`). Nothing happens when no gate applies.
pub(crate) fn withhold_from_caller(
    lookup: &dyn ProviderTerms,
    response: &mut UniversalResponse,
    read_side: Withheld,
    exposure: Option<Exposure>,
) {
    let Some(exposure) = exposure else {
        return;
    };
    let mut withheld = read_side;
    if let Some(result) = response.result.as_mut() {
        withheld.merge(filter_json(lookup, result, exposure));
    }
    let mut notes = Map::new();
    if withheld.from_model() && exposure.to_model {
        notes.insert(WITHHELD_KEY.to_owned(), withheld.to_note());
    }
    if withheld.off_interface > 0 {
        notes.insert(UNAVAILABLE_HERE_KEY.to_owned(), withheld.interface_note());
    }
    if notes.is_empty() {
        return;
    }
    // The notes ride in the result: the chat loop and MCP clients read the
    // result, never `metadata`. A result that is not an object is wrapped,
    // which only ever happens to a result that had something held back.
    match response.result.take() {
        Some(Value::Object(mut object)) => {
            object.extend(notes);
            response.result = Some(Value::Object(object));
        }
        other => {
            notes.insert("result".to_owned(), other.unwrap_or(Value::Null));
            response.result = Some(Value::Object(notes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{Activity, ActivityBuilder, SportType};
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_providers::ai_scope::{
        ai_read, filter_activities, for_display, result_exposure, serve_over,
    };
    use pierre_providers::provider_terms::{NOLIO, NOLIO_TRANSPORT, WHOOP};
    use serde_json::json;

    struct Lookup;

    impl ProviderTerms for Lookup {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "nolio" => Some(&NOLIO),
                "whoop" => Some(&WHOOP),
                _ => None,
            }
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            match provider {
                "nolio" => Some(NOLIO_TRANSPORT),
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
        let ((), read_side) = serve_over(
            Transport::WebApp,
            ai_read(async {
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
            }),
        )
        .await;

        let mut response = response(json!({ "activity_list": "1 ride" }));
        withhold_from_caller(&Lookup, &mut response, read_side, Some(Exposure::MODEL));
        let result = response.result.as_ref().unwrap();
        let note = &result[WITHHELD_KEY];
        assert_eq!(note["items_dropped"], 1);
        assert_eq!(note["items_reduced"], 1);
        assert_eq!(note["sources"], json!(["strava", "zepp"]));
        assert!(result.get(UNAVAILABLE_HERE_KEY).is_none());
    }

    #[tokio::test]
    async fn an_external_result_loses_first_party_only_items_and_says_so_neutrally() {
        let ((), read_side) = serve_over(
            Transport::McpHttp,
            ai_read(async {
                let kept = filter_activities(&Lookup, vec![activity("g", Some("garmin"))]);
                assert!(kept.is_empty(), "a Nolio relay item never leaves over MCP");
            }),
        )
        .await;

        let mut response = response(json!({
            "activities": [{ "provider": "nolio", "source": "garmin", "name": "Hills" }],
            "activity_list": "1 ride"
        }));
        withhold_from_caller(
            &Lookup,
            &mut response,
            read_side,
            result_exposure(Some(Transport::McpHttp)),
        );
        let result = response.result.expect("a result");
        assert_eq!(result["activities"], json!([]));
        let note = &result[UNAVAILABLE_HERE_KEY];
        assert_eq!(
            note["items_unavailable"], 2,
            "one read-side, one at the boundary"
        );
        assert!(
            !note.to_string().contains("nolio"),
            "the note never names the service: {note}"
        );
        assert!(
            result.get(WITHHELD_KEY).is_none(),
            "nothing reached the AI rules"
        );
    }

    #[tokio::test]
    async fn outside_an_ai_read_nothing_is_filtered() {
        let kept = filter_activities(&Lookup, vec![activity("z", Some("zepp"))]);
        assert_eq!(kept.len(), 1, "the athlete's own surfaces see everything");
    }

    #[tokio::test]
    async fn display_lifts_the_ai_rules_and_keeps_the_transport_gate() {
        let (kept, withheld) = serve_over(
            Transport::WebApp,
            for_display(ai_read(async {
                filter_activities(&Lookup, vec![activity("z", Some("zepp"))])
            })),
        )
        .await;
        assert_eq!(kept.len(), 1);

        let mut payload = response(json!([{ "provider": "nolio", "source": "zepp" }]));
        let exposure = for_display(async { result_exposure(Some(Transport::WebApp)) }).await;
        assert_eq!(exposure, None, "a first-party chart crosses no gate");
        withhold_from_caller(&Lookup, &mut payload, withheld, exposure);
        assert_eq!(
            payload.result,
            Some(json!([{ "provider": "nolio", "source": "zepp" }]))
        );

        let mut external = response(json!([{ "provider": "nolio", "source": "zepp" }]));
        let exposure = for_display(async { result_exposure(Some(Transport::A2a)) }).await;
        withhold_from_caller(&Lookup, &mut external, Withheld::default(), exposure);
        let result = external.result.expect("a result");
        assert_eq!(result["result"], json!([]));
        assert_eq!(result[UNAVAILABLE_HERE_KEY]["items_unavailable"], 1);
    }

    #[test]
    fn an_array_result_is_wrapped_so_the_model_reads_its_note() {
        let mut payload = response(json!([
            { "provider": "whoop", "recovery_score": 40.0, "hrv_ms": 60.0 }
        ]));
        withhold_from_caller(
            &Lookup,
            &mut payload,
            Withheld::default(),
            Some(Exposure::MODEL),
        );
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
        withhold_from_caller(
            &Lookup,
            &mut payload,
            Withheld::default(),
            Some(Exposure::EXTERNAL_MODEL),
        );
        assert_eq!(payload.result, Some(original));
        assert!(payload.metadata.is_none());
    }
}
