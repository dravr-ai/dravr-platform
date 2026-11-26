// ABOUTME: To handle raises planned_sessions_missed on a Dravr plan's missed days, read live on the roster (carnet#1792)
// ABOUTME: Two missed planned days across a rest day raise it; an isolated miss, or a session done a day late, raise nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::Value;
use uuid::Uuid;

use crate::common::create_test_server_resources;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get, group_agent, join_at, person, plan_with_days, ride, router, row,
    seed_activities, Person,
};

const ROSTER: &str = "/api/coach/roster";

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
    /// A sharer who joined a month ago, whose Dravr plan rides on each of
    /// `planned` days ago and who rode on each of `rode` days ago.
    async fn athlete(&self, name: &str, planned: &[i64], rode: &[i64]) -> Person {
        let email = format!("{}@{}.test", name.to_lowercase(), self.coach.id.simple());
        let athlete = person(&self.res, &email, name).await;
        join_at(
            &self.res,
            self.group,
            athlete.id,
            athlete.tenant,
            true,
            Timestamp::now() - Elapsed::days(30),
        )
        .await;
        let rides: Vec<_> = rode
            .iter()
            .map(|days| ride(&format!("{name}-{days}"), *days))
            .collect();
        seed_activities(&self.res, &athlete, "strava", &rides).await;
        let days: Vec<(CivilDate, &str)> =
            planned.iter().map(|days| (ago(*days), "ride")).collect();
        let first = planned.iter().max().copied().unwrap_or(0);
        plan_with_days(&self.res, (athlete.id, athlete.tenant), ago(first), &days).await;
        athlete
    }

    async fn roster(&self) -> Value {
        let (status, page) = get(&router(&self.res), ROSTER, &self.coach.auth).await;
        assert_eq!(status, 200, "{page:#}");
        page
    }
}

/// The athlete's civil day `days` ago; the fixture athletes keep UTC.
fn ago(days: i64) -> CivilDate {
    Timestamp::now().utc_date() - Elapsed::days(days)
}

fn missed(page: &Value, id: Uuid) -> Option<Value> {
    row(page, id).expect("athlete on the roster")["attention"]
        .as_array()
        .expect("every row carries attention")
        .iter()
        .find(|reason| reason["kind"] == "planned_sessions_missed")
        .cloned()
}

#[tokio::test]
async fn two_missed_planned_days_across_a_rest_day_are_this_weeks_first_reason() {
    let s = squad("missed-run").await;
    // Planned 7, 5 and 3 days ago; ridden 6 days ago, a day after the first
    // plan day: that one was done late, the next two were not done at all.
    let mia = s.athlete("Mia", &[7, 5, 3], &[6]).await;

    let page = s.roster().await;
    let reason = missed(&page, mia.id).unwrap_or_else(|| panic!("no reason raised: {page:#}"));
    assert_eq!(reason["horizon"], "week", "{reason:#}");
    assert_eq!(
        reason["key"], "coachHome.reason.plannedSessionsMissed",
        "bug: the reason's line key changed: {reason:#}"
    );
    assert_eq!(reason["params"]["count"], 2, "{reason:#}");
    assert_eq!(reason["params"]["missed"], 2, "{reason:#}");
    assert_eq!(reason["params"]["planned"], 3, "{reason:#}");
    assert_eq!(reason["params"]["date"], ago(7).to_string(), "{reason:#}");
    let evidence = reason["evidence"].as_array().unwrap();
    let unmatched: Vec<&Value> = evidence
        .iter()
        .filter(|line| line["key"] == "coachHome.evidence.plannedSessionUnmatched")
        .collect();
    assert_eq!(unmatched.len(), 2, "{reason:#}");
    assert_eq!(
        unmatched[0]["params"]["date"],
        ago(5).to_string(),
        "bug: the first missed day is not the plan day five days ago: {reason:#}"
    );
    assert_eq!(
        unmatched[1]["params"]["date"],
        ago(3).to_string(),
        "bug: the second missed day is not the plan day three days ago: {reason:#}"
    );
    assert_eq!(unmatched[0]["source"]["kind"], "plan", "{reason:#}");
    assert!(
        evidence
            .iter()
            .all(|line| line["key"] != "coachHome.evidence.wellnessNote"),
        "bug: a note was read without the athlete's wellness grant: {reason:#}"
    );
    let first = &row(&page, mia.id).unwrap()["attention"][0];
    assert_eq!(
        first["kind"], "planned_sessions_missed",
        "bug: missed sessions are not first of the week: {page:#}"
    );
}

#[tokio::test]
async fn an_isolated_miss_or_a_session_done_a_day_late_raises_nothing() {
    let s = squad("missed-none").await;
    // Planned 7, 5 and 3 days ago: the middle one missed, the others done.
    let isolated = s.athlete("Ines", &[7, 5, 3], &[7, 3]).await;
    // Each planned ride done the day after it.
    let late = s.athlete("Leo", &[7, 5, 3], &[6, 4, 2]).await;
    // Yesterday's ride is still inside its grace day: only one miss is
    // judged, on the day before.
    let recent = s.athlete("Remi", &[3, 1], &[]).await;

    let page = s.roster().await;
    for athlete in [isolated, late, recent] {
        assert_eq!(missed(&page, athlete.id), None, "{page:#}");
    }
}

#[tokio::test]
async fn an_activity_read_stopped_at_its_limit_judges_nothing() {
    let s = squad("missed-cut").await;
    // Planned 7, 5 and 3 days ago and ridden on none of them: a run alone.
    let cora = s.athlete("Cora", &[7, 5, 3], &[]).await;
    // Two hundred rides a fortnight ago fill the check's read to its limit,
    // newest first: what it left out could be the plan days' own rides.
    let rides: Vec<_> = (0..200)
        .map(|i| ride(&format!("Cora-cut-{i}"), 14))
        .collect();
    s.res
        .common
        .repos
        .activity_cache
        .upsert_activities(cora.id, &cora.tenant, "strava", &rides)
        .await
        .unwrap();

    let page = s.roster().await;
    assert_eq!(
        missed(&page, cora.id),
        None,
        "bug: a read stopped at its limit was judged as if complete: {page:#}"
    );
}
