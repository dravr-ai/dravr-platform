// ABOUTME: To handle raises unusual_break from the athlete's own last weeks of training, only under routine detection (carnet#1792 P3b)
// ABOUTME: A reading replaces the five-day rule; no grant, a plan in the window or too little history keeps that rule

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use pierre_core::constants::oauth::providers::INTERVALS_ICU;
use pierre_core::models::{DataSource, DeviceType, StoredRecoveryMetrics};
use pierre_database::repositories::WellnessWriteMode;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;
use uuid::Uuid;

use crate::common::create_test_server_resources;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get, group_agent, join_at, person, plan_with_days, ride, router, row,
    seed_activities, Person,
};

const ROSTER: &str = "/api/coach/roster?limit=100";

struct Squad {
    res: Arc<ServerContext>,
    coach: Person,
    group: Uuid,
}

async fn squad(tag: &str) -> Squad {
    let res = create_test_server_resources().await.unwrap();
    let coach = coach(&res, &format!("casey@{tag}.test"), "Casey Coach").await;
    let agent = group_agent(&res, &coach).await;
    let group = coached_group(&res, coach.tenant, coach.id, &agent, "Squad").await;
    Squad { res, coach, group }
}

impl Squad {
    /// A sharer who joined three months ago and rode on each of `rode` days
    /// ago, who granted the coach routine detection when `granted`.
    async fn athlete(&self, name: &str, rode: &[i64], granted: bool) -> Person {
        let email = format!("{}@{}.test", name.to_lowercase(), self.coach.id.simple());
        let athlete = person(&self.res, &email, name).await;
        join_at(
            &self.res,
            self.group,
            athlete.id,
            athlete.tenant,
            true,
            Timestamp::now() - Elapsed::days(90),
        )
        .await;
        let rides: Vec<_> = rode
            .iter()
            .map(|days| ride(&format!("{name}-{days}"), *days))
            .collect();
        seed_activities(&self.res, &athlete, "strava", &rides).await;
        if granted {
            self.res
                .common
                .repos
                .groups
                .set_coach_routine_detection(
                    &self.group.to_string(),
                    athlete.id,
                    true,
                    Timestamp::now(),
                )
                .await
                .unwrap()
                .expect("the athlete is a live member");
        }
        athlete
    }

    async fn roster(&self) -> Value {
        let (status, page) = get(&router(&self.res), ROSTER, &self.coach.auth).await;
        assert_eq!(
            status, 200,
            "bug: the coach cannot read their roster: {page:#}"
        );
        page
    }

    /// `athlete` shares their wellness with the coach and noted `note` on
    /// `day`.
    async fn noted(&self, athlete: &Person, day: CivilDate, note: &str) {
        let repos = &self.res.common.repos;
        repos
            .groups
            .set_coach_consents(
                &self.group.to_string(),
                athlete.id,
                None,
                Some(true),
                Timestamp::now(),
            )
            .await
            .unwrap()
            .expect("the athlete is a live member");
        let source = repos
            .data_sources
            .upsert_data_source(
                &athlete.tenant,
                &DataSource {
                    id: String::new(),
                    user_id: athlete.id.to_string(),
                    provider: INTERVALS_ICU.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Band,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        let now = Timestamp::now();
        repos
            .recovery
            .upsert_recovery_metrics(
                &athlete.tenant,
                &StoredRecoveryMetrics {
                    id: format!("icu-day-{}", Uuid::new_v4()),
                    user_id: athlete.id.to_string(),
                    data_source_id: source,
                    date: day,
                    recovery_score: None,
                    readiness_score: Some(61),
                    hrv_ms: None,
                    hrv_rmssd: None,
                    resting_heart_rate: None,
                    stress_score: None,
                    body_battery: None,
                    spo2: None,
                    respiratory_rate: None,
                    skin_temp_deviation: None,
                    daily_strain: None,
                    athlete_note: Some(note.to_owned()),
                    source_name: INTERVALS_ICU.to_owned(),
                    recorded_at: now,
                },
                WellnessWriteMode::FullDay,
                now,
            )
            .await
            .unwrap();
    }
}

/// The athlete's civil day `days` ago; the fixture athletes keep UTC.
fn ago(days: i64) -> CivilDate {
    Timestamp::now().utc_date() - Elapsed::days(days)
}

/// Rides every other day from `from` days ago through `to` days ago.
fn every_other_day(from: i64, to: i64) -> Vec<i64> {
    (to..=from).rev().step_by(2).collect()
}

fn kinds(page: &Value, id: Uuid) -> Vec<String> {
    row(page, id).expect("athlete on the roster")["attention"]
        .as_array()
        .expect("every row carries attention")
        .iter()
        .map(|reason| reason["kind"].as_str().unwrap().to_owned())
        .collect()
}

fn reason(page: &Value, id: Uuid, kind: &str) -> Value {
    row(page, id).expect("athlete on the roster")["attention"]
        .as_array()
        .unwrap()
        .iter()
        .find(|reason| reason["kind"] == kind)
        .cloned()
        .unwrap_or_else(|| panic!("no {kind} for the athlete: {page:#}"))
}

#[tokio::test]
async fn a_break_longer_than_any_before_it_is_an_unusual_break_not_quiet_days() {
    let s = squad("break-unusual").await;
    // Every other day from six weeks ago, then nothing for the last week.
    let rode = every_other_day(42, 8);
    let lea = s.athlete("Lea", &rode, true).await;

    let page = s.roster().await;
    assert_eq!(
        kinds(&page, lea.id),
        ["unusual_break"],
        "bug: the athlete's own pattern must replace the five-day rule: {page:#}"
    );
    let found = reason(&page, lea.id, "unusual_break");
    assert_eq!(
        found["horizon"], "week",
        "bug: an unusual break is not this week's: {found:#}"
    );
    assert_eq!(
        found["key"], "coachHome.reason.unusualBreak",
        "bug: the reason reads under another key: {found:#}"
    );
    assert_eq!(
        found["params"]["days"], 7,
        "bug: the break is the whole days since the last ride, today left out: {found:#}"
    );
    let evidence = found["evidence"].as_array().unwrap();
    assert_eq!(
        evidence[0]["key"], "coachHome.evidence.breakLongestBefore",
        "bug: the longest earlier break is not the first line: {found:#}"
    );
    assert_eq!(
        evidence[0]["params"]["days"], 1,
        "bug: every other day leaves one-day breaks: {found:#}"
    );
    let last = evidence
        .iter()
        .find(|line| line["key"] == "coachHome.evidence.lastTrainingDay")
        .unwrap_or_else(|| panic!("no last training day: {found:#}"));
    assert_eq!(
        last["params"]["date"],
        ago(8).to_string(),
        "bug: the last training day is not the last ride: {found:#}"
    );
    assert!(
        evidence
            .iter()
            .all(|line| line["key"] != "coachHome.evidence.wellnessNote"),
        "bug: a note was read without the athlete's wellness grant: {found:#}"
    );
}

#[tokio::test]
async fn a_break_usual_for_the_athlete_raises_nothing_past_five_days() {
    let s = squad("break-usual").await;
    // A ride every nine days for two months: eight-day breaks are this
    // athlete's habit, so seven quiet days are nothing to flag.
    let rode: Vec<i64> = (0..7).map(|week| 7 + 9 * week).collect();
    let noah = s.athlete("Noah", &rode, true).await;

    let page = s.roster().await;
    assert_eq!(
        kinds(&page, noah.id),
        Vec::<String>::new(),
        "bug: a break usual for the athlete still raised a reason: {page:#}"
    );
}

#[tokio::test]
async fn without_the_grant_or_enough_history_the_five_day_rule_stands() {
    let s = squad("break-rule").await;
    let rode = every_other_day(42, 8);
    let ungranted = s.athlete("Ugo", &rode, false).await;
    // Three weeks of history: under the four the baseline needs.
    let recent = s.athlete("Rita", &every_other_day(20, 8), true).await;

    let page = s.roster().await;
    for athlete in [&ungranted, &recent] {
        assert_eq!(
            kinds(&page, athlete.id),
            ["nothing_received"],
            "bug: without a reading the population threshold must stand: {page:#}"
        );
    }
}

#[tokio::test]
async fn an_athlete_with_a_session_planned_is_judged_by_the_plan_alone() {
    let s = squad("break-planned").await;
    let rode = every_other_day(42, 8);
    let pia = s.athlete("Pia", &rode, true).await;
    // A Dravr plan with a ride three days ago, not done.
    plan_with_days(&s.res, (pia.id, pia.tenant), ago(3), &[(ago(3), "ride")]).await;

    let page = s.roster().await;
    assert!(
        !kinds(&page, pia.id).contains(&"unusual_break".to_owned()),
        "bug: the baseline ran beside a plan: {page:#}"
    );
    assert!(
        kinds(&page, pia.id).contains(&"nothing_received".to_owned()),
        "bug: with the plan as the measure, the five-day rule must stand: {page:#}"
    );
}

#[tokio::test]
async fn under_the_wellness_grant_an_unusual_break_quotes_the_note_since_the_last_ride() {
    let s = squad("break-note").await;
    let rode = every_other_day(42, 8);
    let lea = s.athlete("Lea", &rode, true).await;
    s.noted(&lea, ago(3), "sick").await;
    // A note written before the last ride says nothing of the break.
    let ines = s.athlete("Ines", &rode, true).await;
    s.noted(&ines, ago(10), "travelling").await;

    let page = s.roster().await;
    let notes = |id: Uuid| -> Vec<Value> {
        reason(&page, id, "unusual_break")["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|line| line["key"] == "coachHome.evidence.wellnessNote")
            .cloned()
            .collect()
    };
    let quoted = notes(lea.id);
    assert_eq!(
        quoted.len(),
        1,
        "bug: the granted note since the last ride was not quoted: {page:#}"
    );
    assert_eq!(
        (&quoted[0]["params"]["note"], &quoted[0]["params"]["date"]),
        (&Value::from("sick"), &Value::from(ago(3).to_string())),
        "bug: the note was not quoted as written on its day: {page:#}"
    );
    assert_eq!(
        notes(ines.id),
        Vec::<Value>::new(),
        "bug: a note from before the last ride was quoted: {page:#}"
    );
}
