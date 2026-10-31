// ABOUTME: `pierre-cli seed coach-home` leaves a coach whose roster passes forty athletes with every attention reason and season state
// ABOUTME: Read back through GET /api/coach/roster as the live walk of Coach Home reads it, and idempotent on a rerun
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The Coach Home walk (carnet#1253) needs a coach holding more than forty
//! athletes in which every reason the queue raises and every section the
//! season view draws appears. A seeder whose scenarios drift from what the
//! server computes would leave the walk with an empty queue and nothing
//! failing, so the seeded roster is read here the way the screens read it.

#[cfg(feature = "tools-groups")]
mod coach_home_seed_tests {
    use std::collections::BTreeSet;

    use pierre_seeders::coach_home::{run, SeedArgs};
    use serde_json::Value;

    use crate::common::create_test_server_resources;
    use crate::helpers::coach_roster_fixtures::{get, person, router};

    const COACH: &str = "coach@home-seed.test";

    fn args() -> SeedArgs {
        SeedArgs {
            coach_email: COACH.to_owned(),
            model: Some("seed-model".to_owned()),
        }
    }

    fn strings<'a>(page: &'a Value, pick: impl Fn(&'a Value) -> Vec<&'a str>) -> BTreeSet<&'a str> {
        page["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(pick)
            .collect()
    }

    #[tokio::test]
    async fn the_seeded_coach_holds_every_reason_and_season_state_past_forty_athletes() {
        let res = create_test_server_resources().await.unwrap();
        let coach = person(&res, COACH, "Alice Coach").await;
        let repos = &res.common.repos;
        run(args(), repos).await.unwrap();

        let (status, page) = get(&router(&res), "/api/coach/roster?limit=100", &coach.auth).await;
        assert_eq!(status, 200, "{page:#}");
        assert_eq!(page["scope"]["roster_members"], 45, "{:#}", page["scope"]);
        assert_eq!(page["groups"].as_array().unwrap().len(), 3);

        let kinds = strings(&page, |a| {
            a["attention"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| r["kind"].as_str().unwrap())
                .collect()
        });
        assert_eq!(
            kinds,
            BTreeSet::from([
                "consent_revoked",
                "deep_fatigue_streak",
                "new_member_no_plan",
                "nothing_received",
                "plan_draft_waiting",
                "plan_ended",
                "provider_reconnect",
                "unverified_figure",
            ]),
            "every reason the queue raises appears"
        );
        let horizons = strings(&page, |a| {
            a["attention"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|r| r["kind"] == "nothing_received")
                .map(|r| r["horizon"].as_str().unwrap())
                .collect()
        });
        assert_eq!(
            horizons,
            BTreeSet::from(["today", "week"]),
            "a dead source today, quiet live sources this week"
        );
        let seasons = strings(&page, |a| {
            a["season"]["state"].as_str().into_iter().collect()
        });
        assert_eq!(
            seasons,
            BTreeSet::from([
                "a_race_ahead",
                "draft_only",
                "new_athlete",
                "no_a_race",
                "no_plan",
                "nothing_since_last_race",
                "plan_ended",
            ]),
            "every section of the season view has an athlete"
        );
        let statuses = strings(&page, |a| vec![a["status"].as_str().unwrap()]);
        assert_eq!(
            statuses,
            BTreeSet::from(["no_coach_consent", "no_source", "shared"])
        );
        let calm = page["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["attention"].as_array().unwrap().is_empty())
            .count();
        assert!(calm > 10, "most of the squad is calm: {calm}");

        // The fatigue athletes sit just inside the band, as a real loading
        // block leaves them, not at numbers no athlete reads.
        let athletes = page["athletes"].as_array().unwrap();
        let mornings: Vec<f64> = athletes
            .iter()
            .flat_map(|a| a["attention"].as_array().unwrap().clone())
            .filter(|r| r["kind"] == "deep_fatigue_streak")
            .flat_map(|r| r["evidence"].as_array().unwrap().clone())
            .filter(|e| e["key"] == "coachHome.evidence.formMorning")
            .map(|e| e["params"]["form_pct"].as_f64().unwrap())
            .collect();
        assert!(
            mornings.len() >= 9,
            "three athletes, three mornings or more each: {mornings:?}"
        );
        assert!(
            mornings.iter().all(|pct| (-40.0..-30.0).contains(pct)),
            "just inside deep fatigue: {mornings:?}"
        );

        // Plans differ by group and athlete, so the season view is no column
        // of copies.
        let goals: BTreeSet<(&str, &str)> = athletes
            .iter()
            .filter(|a| a["season"]["state"] == "a_race_ahead")
            .map(|a| {
                (
                    a["plan"]["goal_race"]["name"].as_str().unwrap(),
                    a["plan"]["goal_race"]["date"].as_str().unwrap(),
                )
            })
            .collect();
        assert!(
            goals.len() > 15,
            "{} distinct A races: {goals:?}",
            goals.len()
        );
        let phases = strings(&page, |a| a["plan"]["phase"].as_str().into_iter().collect());
        for phase in ["base", "build", "specialty", "taper", "transition"] {
            assert!(
                phases.contains(phase),
                "{phase} is current for someone: {phases:?}"
            );
        }

        // A rerun leaves the coach as they are.
        run(args(), repos).await.unwrap();
        let (_, again) = get(&router(&res), "/api/coach/roster?limit=100", &coach.auth).await;
        assert_eq!(again["scope"]["roster_members"], 45);
        assert_eq!(again["groups"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn a_coach_the_seeder_creates_is_a_roster_manager_not_an_admin() {
        let res = create_test_server_resources().await.unwrap();
        let repos = &res.common.repos;
        run(args(), repos).await.unwrap();

        let coach = repos.users.get_by_email_required(COACH).await.unwrap();
        assert!(
            !coach.is_admin,
            "an admin lands on the admin console, never on Coach Home"
        );
        assert!(coach.manages_roster, "the coach reads their roster");
        assert!(
            repos.users.coaches_others(coach.id).await.unwrap(),
            "the coach answered that they coach others"
        );
        let steps = repos
            .user_onboarding
            .get_onboarding_steps(&coach.id.to_string())
            .await
            .unwrap();
        for step in [
            "profile_type",
            "about_you",
            "parq",
            "check_in",
            "coach_group",
        ] {
            assert!(
                steps.iter().any(|s| s.step_id == step),
                "a returning coach has answered {step}: {steps:?}"
            );
        }
    }
}
