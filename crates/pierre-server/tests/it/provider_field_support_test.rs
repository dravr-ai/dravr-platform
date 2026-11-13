// ABOUTME: Pins the field dictionary: every activity backend declares its fields, both modes of a provider are joined
// ABOUTME: get_connection_status carries a provider's dictionary and get_activities what its serving connection never supplies
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Field dictionary suite (carnet#1494).
//
// This `//!` must precede the crate-level `#![cfg]`: when a feature is off the
// cfg empties the module, and the surviving doc keeps `missing_docs` quiet.
#![cfg(all(
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-sciotte"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The field dictionary (directive 6): a function of the
//! provider, per mode (mirror and OAuth), that the model reads so it never
//! takes a field a connection does not supply for a zero.
//!
//! Each mapper's own unit test checks that its fixture fills what its
//! descriptor declares. This suite pins what crosses crates: that every
//! backend the registry builds for activities declares something, how a
//! provider's two modes join, and that both tool answers carry the
//! dictionary to the model, in the words `backend` uses.

use std::sync::Arc;

use chrono::{TimeZone, Utc};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_providers::field_support::{field_support, ActivityFields, FieldMode};
use pierre_providers::ProviderRegistry;
use pierre_tool_runtime::implementations::field_dictionary::served_header;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalToolExecutor};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_results::{athlete_notes, project_activities_payload};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::sciotte_mock::seed_scrape_session;

/// What Strava's API supplies and its scrape does not: the normalized power,
/// the route's summary polyline and the finer sport label.
const API_ONLY: ActivityFields = ActivityFields::NORMALIZED_POWER
    .union(ActivityFields::SUMMARY_POLYLINE)
    .union(ActivityFields::SPORT_TYPE_DETAIL);

/// What the scrape of Strava's pages reads since sciotte 2.0, as the API
/// does: the athlete's words, the workout tag and the trainer and commute
/// flags.
const READ_BOTH_WAYS: ActivityFields = ActivityFields::DESCRIPTION
    .union(ActivityFields::PRIVATE_NOTE)
    .union(ActivityFields::WORKOUT_TYPE)
    .union(ActivityFields::TRAINER)
    .union(ActivityFields::COMMUTE);

#[test]
fn every_activity_backend_the_registry_builds_declares_its_fields() {
    let registry = ProviderRegistry::new();
    let mut declared = Vec::new();
    for name in registry.supported_providers() {
        let descriptor = registry.get_descriptor(name).unwrap();
        if !descriptor.capabilities().supports_activities() {
            continue;
        }
        assert!(
            !descriptor.activity_fields().is_empty(),
            "bug: {name} serves activities and promises no field: declare what its mapper fills"
        );
        declared.push(name);
    }
    for backend in [
        "strava",
        "sciotte",
        "garmin",
        "sciotte_garmin",
        "intervals_icu",
    ] {
        assert!(
            declared.contains(&backend),
            "the shipped backend {backend} is checked: {declared:?}"
        );
    }
}

#[test]
fn strava_is_read_two_ways_and_both_carry_the_athletes_words_and_flags() {
    let registry = ProviderRegistry::new();
    let support = field_support(&registry, "strava", Some(FieldMode::Mirror)).unwrap();
    assert_eq!(
        support.provider, "strava",
        "bug: Strava's dictionary is filed under another provider"
    );
    let api = support.oauth.expect("Strava's API serves activities");
    let scrape = support.mirror.expect("Strava's mirror serves activities");
    assert!(
        api.supplied().contains(API_ONLY | READ_BOTH_WAYS),
        "the API supplies the description, private note, workout tag and flags"
    );
    assert!(
        scrape.not_supplied().contains(API_ONLY),
        "bug: the pinned scrape is declared to supply a field it never reads"
    );
    assert!(
        scrape.supplied().contains(READ_BOTH_WAYS),
        "bug: the pinned scrape reads the athlete's words and flags, and does not declare them"
    );
    assert!(
        scrape.detail_only.contains(ActivityFields::PRIVATE_NOTE),
        "a scraped Strava row carries the private note only once read in detail"
    );
    assert_eq!(support.served(), Some(scrape), "the session serves");

    let by_backend = field_support(&registry, "sciotte", Some(FieldMode::Mirror)).unwrap();
    assert_eq!(
        by_backend, support,
        "a mirror backend's name reads as the provider it mirrors"
    );
}

#[test]
fn a_provider_read_one_way_has_only_that_mode() {
    let registry = ProviderRegistry::new();

    let trainingpeaks = field_support(&registry, "trainingpeaks", Some(FieldMode::Oauth)).unwrap();
    assert!(trainingpeaks.mirror.is_some(), "TrainingPeaks is scraped");
    assert!(
        trainingpeaks.oauth.is_none(),
        "TrainingPeaks has no API Dravr reads"
    );
    assert_eq!(
        trainingpeaks.serving, None,
        "a mode the provider lacks never serves"
    );

    let intervals = field_support(&registry, "intervals_icu", Some(FieldMode::Oauth)).unwrap();
    assert!(intervals.mirror.is_none(), "Intervals.icu has no mirror");
    let api = intervals.served().unwrap();
    assert!(
        api.list.contains(
            ActivityFields::TRAINER | ActivityFields::COMMUTE | ActivityFields::DESCRIPTION
        ),
        "Intervals.icu carries the flags and the description on every row"
    );

    assert_eq!(
        field_support(&registry, "peloton", None),
        None,
        "an unknown name has no dictionary"
    );
}

/// TrainingPeaks and COROS keep the athlete's words as comments (directive
/// 10): both declare them, and a row's comments survive the projection the
/// model reads and the notes the prefetch appends.
#[test]
fn trainingpeaks_and_coros_comments_reach_the_model() {
    let registry = ProviderRegistry::new();
    for provider in ["trainingpeaks", "coros"] {
        let support = field_support(&registry, provider, Some(FieldMode::Mirror)).unwrap();
        let mirror = support.mirror.expect("the provider is scraped");
        assert!(
            mirror.supplied().contains(ActivityFields::COMMENTS),
            "bug: {provider} does not declare the comments its capture carries"
        );
    }

    let envelope = json!({
        "activity_list": "1. [run] Seuil - 2026-11-08 …",
        "activities": [{
            "id": "tp-1",
            "name": "Seuil",
            "sport_type": "run",
            "start_date": "2026-11-08T12:00:00Z",
            "comments": [{ "author": "coach", "text": "<athlete_text>Bien géré</athlete_text>" }]
        }]
    });
    let projected = project_activities_payload("get_activities", &envelope)
        .expect("a real get_activities envelope is recognised");
    assert_eq!(
        projected["activities"][0]["comments"], envelope["activities"][0]["comments"],
        "bug: the projection dropped the comments the model needs"
    );
    let notes = athlete_notes(&envelope);
    assert!(
        notes.len() == 1 && notes[0].contains("coach: <athlete_text>Bien géré"),
        "bug: the prefetch notes dropped a session's comments: {notes:?}"
    );
}

fn row(id: &str, provider: &str, source: &str) -> Activity {
    ActivityBuilder::new(
        id.to_owned(),
        "Session".to_owned(),
        SportType::Ride,
        Utc.with_ymd_and_hms(2026, 11, 7, 13, 0, 0).unwrap(),
        3_600,
        provider.to_owned(),
    )
    .source(source)
    .build()
}

#[test]
fn the_activities_header_names_what_each_serving_connection_never_supplies() {
    let rows = vec![
        row("1", "sciotte", "strava"),
        row("2", "strava", "strava"),
        row("3", "sciotte", "trainingpeaks"),
        row("4", "sciotte", "strava"),
    ];
    let header = served_header(&rows).expect("the rows' connections declare their fields");
    let connections = serde_json::to_value(&header.connections).unwrap();
    let keyed: Vec<(String, String)> = connections
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["provider"].as_str().unwrap().to_owned(),
                c["via"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        keyed,
        [
            ("strava".to_owned(), "mirror".to_owned()),
            ("strava".to_owned(), "oauth".to_owned()),
            ("trainingpeaks".to_owned(), "mirror".to_owned()),
        ],
        "one entry per connection that served rows, never a backend key"
    );
    let not_supplied = |index: usize| -> Vec<&str> {
        connections[index]["not_supplied"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
            .collect()
    };
    for field in API_ONLY.names() {
        assert!(
            not_supplied(0).contains(&field),
            "the scrape's rows never carry {field}: {connections:#}"
        );
        assert!(
            !not_supplied(1).contains(&field),
            "the API's rows can carry {field}: {connections:#}"
        );
    }
    assert!(
        connections[0]["detail_only"]
            .as_array()
            .unwrap()
            .contains(&json!("max_heart_rate")),
        "a scraped Strava row carries its maximum heart rate only once read in detail"
    );

    let payload = json!({
        "activity_list": "- Session",
        "provider": "strava",
        "count": rows.len(),
        "field_support": header,
    });
    let projected = project_activities_payload("get_activities", &payload).unwrap();
    assert_eq!(
        projected["field_support"], payload["field_support"],
        "bug: the prompt projection drops the header, so no model reads it"
    );
}

#[test]
fn rows_from_no_declared_connection_carry_no_header() {
    assert!(
        served_header(&[]).is_none(),
        "bug: an empty answer carries a field header"
    );
    assert!(
        served_header(&[row("u1", "upload", "upload")]).is_none(),
        "bug: the athlete's own uploads carry a field header"
    );
}

async fn status(
    executor: &UniversalToolExecutor,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: Option<&str>,
) -> Value {
    let parameters = provider.map_or_else(|| json!({}), |p| json!({ "provider": p }));
    let response = executor
        .execute_tool(UniversalRequest {
            user_id: user_id.to_string(),
            tool_name: "get_connection_status".to_owned(),
            parameters,
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant_id.to_string()),
        })
        .await
        .unwrap();
    assert!(response.success, "{:?}", response.error);
    response.result.expect("the status tool answers a payload")
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn connection_status_carries_each_providers_dictionary_and_the_mode_serving() {
    let resources = common::create_test_server_resources().await.unwrap();
    let email = format!("fields-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant_id) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    seed_scrape_session(&resources, user_id, tenant_id, "sciotte", "fields-session").await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = UniversalToolExecutor::new(runtime).with_scopes(OAuthScope::self_grant());

    let strava = status(&executor, user_id, tenant_id, Some("strava")).await;
    assert_eq!(strava["backend"], "mirror", "{strava:#}");
    let fields = &strava["fields"];
    assert_eq!(fields["serving"], "mirror", "{strava:#}");
    for field in API_ONLY.names() {
        assert!(
            !names(&fields["mirror"]["list"]).contains(&field)
                && !names(&fields["mirror"]["detail_only"]).contains(&field),
            "bug: the mirror is said to supply {field}: {fields:#}"
        );
    }
    assert!(
        names(&fields["oauth"]["detail_only"]).contains(&"description"),
        "the API's detail read carries the description: {fields:#}"
    );
    assert!(
        !fields.to_string().contains("sciotte"),
        "bug: an internal backend key reaches the model: {fields:#}"
    );

    let all = status(&executor, user_id, tenant_id, None).await;
    let trainingpeaks = &all["providers"]["trainingpeaks"]["fields"];
    assert!(
        trainingpeaks["mirror"].is_object() && trainingpeaks.get("oauth").is_none(),
        "TrainingPeaks is read one way: {trainingpeaks:#}"
    );
    assert!(
        trainingpeaks.get("serving").is_none(),
        "nothing serves a provider the athlete has not connected: {trainingpeaks:#}"
    );
    assert_eq!(
        all["providers"]["strava"]["fields"], strava["fields"],
        "the map and the single answer agree"
    );
}
