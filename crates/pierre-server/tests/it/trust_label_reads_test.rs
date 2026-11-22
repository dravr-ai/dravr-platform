// ABOUTME: Every read of an athlete's data an MCP caller makes labels what it served, as the trust label's invariant
// ABOUTME: Each tool listed with athlete= is called as the athlete over MCP against stored data; a read with no label fails

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! owl phase 1 (carnet#1539): a caller that writes its own reply is told how
//! sensitive what it was served is, and how far to trust it, in the result's
//! `_meta`. A read that serves the athlete's data and carries no label claims
//! nothing about it, which is the defect this suite exists to catch: it was
//! found read by read (`get_athlete`, stored physiology, `verify_claim`), so
//! the reads are checked as a class.
//!
//! The class is what the registry lists with the shared `athlete` argument:
//! every tool declaring a training or wellness data class, plus the plan
//! tools that take their own. Each one must have a probe here, so a tool that
//! gains a class cannot skip the check, and each read is called as the
//! athlete over MCP against an athlete holding every kind of data it reads.

use std::collections::{BTreeMap, BTreeSet};
use std::slice;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_cache::{CacheKey, CacheResource};
use pierre_core::constants::oauth::providers::STRAVA_ARCHIVE;
use pierre_core::models::{ActivityBuilder, Athlete, DailyTrainingState, SegmentEffort, SportType};
use pierre_tool_runtime::activity_detail_store::store_detail;
use pierre_tool_runtime::protocol::trust_labels::{labels_of, TRUST_LABELS_META_KEY};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};

use crate::coach_athlete_reads_test::{Fixture, ATHLETE_STREAM_RUN, UPLOAD};
use crate::coach_athlete_training_reads_test::{phase_two_reads, world as training_world};
use crate::coach_athlete_wellness_reads_test::{file_wellness, wellness_reads, ATHLETE_WELLNESS};

/// The segment the athlete's stored run carries an effort on.
const SEGMENT_ID: &str = "7781";

/// What a call of one tool is expected to show about its label.
enum Probe {
    /// Called with these arguments, it serves the athlete's stored data, and
    /// its result must carry a label.
    Labels(Value),
    /// Called with these arguments, it serves none of the athlete's items —
    /// for the reason given — and carries no label: no label makes no claim.
    ServesNone(Value, &'static str),
    /// It serves only what a connected provider answers live, which this
    /// world holds none of, so the call is refused; the governed provider's
    /// filter point labels what one serves (`filter_planned_workouts`).
    LiveOnly(&'static str),
    /// It runs only on Dravr's own surfaces, so an MCP caller is never served
    /// it (carnet#1565).
    DravrOnly,
    /// It declares a tier an outside client never reads until grants carry
    /// authorization details, so the dispatch tier gate refuses every MCP
    /// call (carnet#1544); called with these arguments, nothing is served to
    /// label.
    OutsideRefused(Value),
    /// A write naming whose plan, calendar or link it changes; it reads
    /// nothing to label.
    Writes,
}

/// Every tool the registry lists with `athlete`, by what its call shows.
fn probes(goal: &str) -> BTreeMap<&'static str, Probe> {
    let by_id = json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN });
    let mut probes: BTreeMap<&'static str, Probe> = phase_two_reads(goal)
        .into_iter()
        .chain(wellness_reads())
        .map(|(tool, params)| (tool, Probe::Labels(params)))
        .collect();
    for (tool, params) in [
        (
            "get_training_history",
            json!({
                "from": (Timestamp::now() - Elapsed::days(3)).utc_date().to_string(),
                "to": Timestamp::now().utc_date().to_string(),
            }),
        ),
        ("analyze_training_load", json!({ "provider": UPLOAD })),
        ("calculate_fitness_score", json!({ "provider": UPLOAD })),
        (
            "get_activities",
            json!({ "provider": UPLOAD, "limit": 10, "mode": "summary" }),
        ),
        ("analyze_activity", by_id.clone()),
        ("get_activity_intelligence", by_id),
        (
            "extract_activity_streams",
            json!({ "activity_id": ATHLETE_STREAM_RUN }),
        ),
        (
            "export_intervals",
            json!({ "activity_id": ATHLETE_STREAM_RUN }),
        ),
        ("export_latest_snapshot", json!({ "window": 30 })),
        ("get_segment_history", json!({ "segment_id": SEGMENT_ID })),
        ("query_activities", json!({ "provider": UPLOAD })),
        ("calculate_personalized_zones", json!({})),
        (
            "recommend_plan_flavour",
            json!({ "hours_per_week": 6.0, "sessions_per_week": 4 }),
        ),
    ] {
        probes.insert(tool, Probe::Labels(params));
    }
    for (tool, reason) in [
        (
            "list_data_sources",
            "it names the athlete's connections, never an item read from one",
        ),
        (
            "get_data_freshness",
            "it names when each connection last synced, never an item read from one",
        ),
    ] {
        probes.insert(tool, Probe::ServesNone(json!({}), reason));
    }
    probes.insert(
        "get_training_plan",
        Probe::ServesNone(
            json!({}),
            "this athlete holds no plan, calendar entry or draft; the plan suite labels each",
        ),
    );
    probes.insert(
        "get_planned_workouts",
        Probe::LiveOnly("a provider's planned calendar is read live, never stored"),
    );
    for tool in [
        "calculate_daily_nutrition",
        "get_nutrient_timing",
        "get_user_configuration",
    ] {
        probes.insert(tool, Probe::DravrOnly);
    }
    probes.insert(
        "get_health_records",
        Probe::OutsideRefused(json!({ "kind": "labs" })),
    );
    for tool in [
        "save_training_plan",
        "publish_training_plan",
        "push_training_plan",
        "prescribe_workout",
        "withdraw_prescribed_workout",
        "set_preferred_calendar",
        "propose_athlete_link",
    ] {
        probes.insert(tool, Probe::Writes);
    }
    probes
}

/// The training fixture with the rest of what the class reads: the
/// athlete's wellness week, a day of their load history, their provider
/// profile cached under the upload key, and a segment effort on a ride from
/// their Strava export, which any transport may carry.
async fn world() -> (Fixture, String) {
    let (f, goal) = training_world().await;
    file_wellness(&f, f.athlete, ATHLETE_WELLNESS).await;
    let repos = f.repos();
    let mut day = DailyTrainingState::zero(Timestamp::now().utc_date() - Elapsed::days(1));
    day.daily_load = 111.0;
    repos
        .training_history
        .upsert_training_history_batch(f.athlete.tenant, f.athlete.id, &[day])
        .await
        .unwrap();
    let start =
        Timestamp::from_epoch_seconds(Timestamp::now().epoch_seconds() - 5 * 86_400).unwrap();
    let ridden = ActivityBuilder::new(
        "athlete-segment-ride",
        "Hill repeats",
        SportType::Ride,
        start,
        3_600,
        STRAVA_ARCHIVE,
    )
    .distance_meters(30_000.0)
    .segment_efforts(vec![SegmentEffort {
        id: "athlete-segment-ride-effort".to_owned(),
        segment_id: Some(SEGMENT_ID.to_owned()),
        name: "Mont Royal".to_owned(),
        elapsed_time: 412,
        moving_time: Some(412),
        start_date: start + Elapsed::minutes(10),
        distance: 1_820.5,
        average_heart_rate: Some(161),
        max_heart_rate: None,
        average_cadence: None,
        average_watts: Some(287),
        kom_rank: None,
        pr_rank: Some(1),
        climb_category: None,
        average_grade: None,
        elevation_gain: None,
    }])
    .build();
    let runtime: Arc<dyn ToolRuntime> = f.res.clone();
    repos
        .activity_cache
        .upsert_activities(
            f.athlete.id,
            &f.athlete.tenant,
            STRAVA_ARCHIVE,
            slice::from_ref(&ridden),
        )
        .await
        .unwrap();
    assert!(
        store_detail(
            runtime.repos(),
            runtime.live_activities().as_ref(),
            f.athlete.tenant,
            f.athlete.id,
            STRAVA_ARCHIVE,
            &ridden,
        )
        .await,
        "fixture: the ride's detail is stored"
    );
    let profile = CacheKey::new(
        f.athlete.tenant,
        f.athlete.id,
        UPLOAD.to_owned(),
        CacheResource::AthleteProfile,
    );
    f.res
        .data()
        .cache()
        .set(
            &profile,
            &Athlete {
                active_streak: None,
                created_at: None,
                id: "4242".to_owned(),
                username: "mira".to_owned(),
                firstname: Some("Mira".to_owned()),
                lastname: None,
                profile_picture: None,
                provider: UPLOAD.to_owned(),
                sex: None,
            },
            StdDuration::from_secs(600),
        )
        .await
        .unwrap();
    (f, goal)
}

/// Every tool `tools/list` names with the `athlete` argument.
fn listed_with_athlete(f: &Fixture) -> BTreeSet<String> {
    f.res
        .mcp
        .tool_registry
        .all_schemas()
        .into_iter()
        .filter(|schema| {
            schema
                .input_schema
                .properties
                .as_ref()
                .is_some_and(|properties| properties.contains_key("athlete"))
        })
        .map(|schema| schema.name)
        .collect()
}

#[tokio::test]
async fn every_read_of_the_athletes_data_over_mcp_labels_what_it_served() {
    let (f, goal) = world().await;
    let probes = probes(&goal);
    let listed = listed_with_athlete(&f);
    let unprobed: Vec<&String> = listed
        .iter()
        .filter(|tool| !probes.contains_key(tool.as_str()))
        .collect();
    assert!(
        unprobed.is_empty(),
        "a tool listed with athlete= reads a person's data: give it a probe here, so its \
         label is checked: {unprobed:?}"
    );
    let stale: Vec<&&str> = probes
        .keys()
        .filter(|tool| !listed.contains(**tool))
        .collect();
    assert!(
        stale.is_empty(),
        "a probe names a tool this build does not list with athlete=: {stale:?}"
    );

    let mut defects = Vec::new();
    for (tool, probe) in probes {
        let (params, serves_none) = match probe {
            Probe::Labels(params) => (params, None),
            Probe::ServesNone(params, reason) => (params, Some(reason)),
            Probe::DravrOnly => {
                let refused = f
                    .try_call(&f.mcp_executor(), f.athlete, tool, json!({}))
                    .await;
                if refused.as_ref().is_ok_and(|response| response.success) {
                    defects.push(format!("{tool}: served over MCP though Dravr-only"));
                }
                continue;
            }
            Probe::OutsideRefused(params) => {
                let refused = f.try_call(&f.mcp_executor(), f.athlete, tool, params).await;
                if refused.as_ref().is_ok_and(|response| response.success) {
                    defects.push(format!(
                        "{tool}: served over MCP though its tier is refused to outside clients"
                    ));
                }
                continue;
            }
            Probe::LiveOnly(reason) => {
                let answered = f
                    .try_call(&f.mcp_executor(), f.athlete, tool, json!({}))
                    .await;
                if answered.as_ref().is_ok_and(|response| response.success) {
                    defects.push(format!(
                        "{tool}: answered with no live provider ({reason}); give it a Labels probe"
                    ));
                }
                continue;
            }
            Probe::Writes => continue,
        };
        let response = f.call(&f.mcp_executor(), f.athlete, tool, params).await;
        if !response.success {
            defects.push(format!(
                "{tool}: failed, so its label is unchecked: {:?} {:?}",
                response.error, response.result
            ));
            continue;
        }
        match (labels_of(&response), serves_none) {
            (None, None) => defects.push(format!(
                "{tool}: served the athlete's data with no {TRUST_LABELS_META_KEY} label: {}",
                response.result.unwrap_or_default()
            )),
            (Some(label), Some(reason)) => {
                defects.push(format!(
                    "{tool}: labelled though it serves no item ({reason}): {label}"
                ));
            }
            (Some(label), None) => {
                if label["v"] != 1 || label["source_class"].as_array().is_none_or(Vec::is_empty) {
                    defects.push(format!("{tool}: a label naming no source: {label}"));
                }
            }
            (None, Some(_)) => {}
        }
    }
    assert!(
        defects.is_empty(),
        "every read must label what it served:\n{}",
        defects.join("\n")
    );
}

#[tokio::test]
async fn the_athletes_own_physiology_is_labelled_as_theirs() {
    let (f, _) = world().await;
    let response = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "calculate_personalized_zones",
            json!({}),
        )
        .await;
    assert!(response.success, "{:?}", response.error);
    let label = labels_of(&response).expect("the stored physiology was served");
    assert_eq!(
        label["source_class"],
        json!(["patient_entered"]),
        "the athlete stated every number of their profile: {label}"
    );
    assert_eq!(label["tier"], "T1", "{label}");
}

#[tokio::test]
async fn the_athletes_provider_profile_is_labelled_by_its_source() {
    let (f, _) = world().await;
    let response = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "get_athlete",
            json!({ "provider": UPLOAD }),
        )
        .await;
    assert!(response.success, "{:?}", response.error);
    let label = labels_of(&response).expect("the cached profile was served");
    assert!(
        label["source_class"]
            .as_array()
            .is_some_and(|classes| classes.contains(&json!("device"))),
        "an upload is the device's record: {label}"
    );
}

#[tokio::test]
async fn a_claim_judged_against_the_record_is_labelled_and_one_judged_by_rhetoric_is_not() {
    let (f, _) = world().await;
    let judged = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "verify_claim",
            json!({
                "claim": "You ran 42 km yesterday.",
                "category": "athlete_data",
            }),
        )
        .await;
    assert!(judged.success, "{:?}", judged.error);
    let verdict = judged.result.clone().unwrap_or_default();
    assert_eq!(
        verdict["layer_fired"], "athlete_data",
        "the claim is checked against the record: {verdict}"
    );
    let label = labels_of(&judged).expect("the verdict explains itself with the record");
    assert!(
        label["source_class"]
            .as_array()
            .is_some_and(|classes| classes.contains(&json!("device"))),
        "{label}"
    );

    let rhetoric = f
        .call(
            &f.mcp_executor(),
            f.athlete,
            "verify_claim",
            json!({
                "claim": "How did your run feel?",
                "category": "athlete_data",
            }),
        )
        .await;
    assert!(rhetoric.success, "{:?}", rhetoric.error);
    let verdict = rhetoric.result.clone().unwrap_or_default();
    assert_ne!(
        verdict["layer_fired"], "athlete_data",
        "a question is no claim about the record: {verdict}"
    );
    assert_ne!(verdict["layer_fired"], "personalized", "{verdict}");
    assert_eq!(
        labels_of(&rhetoric),
        None,
        "a verdict resting on none of the athlete's data serves none of it: {verdict}"
    );
}
