// ABOUTME: Coach Home's attention reasons on GET /api/coach/roster, and the coach's "Traité" on one (carnet#1252, #1250)
// ABOUTME: One case per reason kind, the dead source, ranking, the non-sharer, dismissal and its reset, tenant isolation, the season view

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The queue a coach lands on is computed on the server, so every rule of it
//! is pinned here at the HTTP boundary: which signal raises which reason,
//! today or this week, in what order, with what evidence and source; that a
//! dead source reads as "nothing received", never as an athlete who stopped;
//! that a non-sharer gets the consent reason and nothing inferred; and that a
//! coach's "Traité" holds until the signal changes, for that coach only.

#[cfg(feature = "tools-groups")]
mod coach_attention_route_tests {
    use std::slice;
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use pierre_core::models::periodization::PhaseKind;
    use pierre_core::models::{Activity, TenantId};
    use pierre_core::transport::TransportPolicy;
    use pierre_database::repositories::training_plans::{DraftOwner, PlanAuthor, SaveDraftParams};
    use pierre_database::repositories::{AttentionDismissalKey, PlanOutlineInput};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_memory::claims::ClaimStatus;
    use pierre_memory::training_plans::{GoalRace, RacePriority};
    use serde_json::{json, Value};
    use uuid::Uuid;

    use crate::common::create_test_server_resources;
    use crate::helpers::coach_roster_fixtures::{
        agent_reply, coach, coached_group, get, group_agent, join, join_at, live_plan, person,
        phase, post, ride, ride_lasting, router, row, seed_activities, seed_ride, ymd, Person,
    };

    const ROSTER: &str = "/api/coach/roster?limit=100";

    /// A coach with one group, ready for athletes.
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
        /// An athlete who joined `days_ago` and shares (or not).
        async fn athlete(&self, name: &str, shares: bool, days_ago: i64) -> Person {
            let email = format!("{}@{}.test", name.to_lowercase(), self.coach.id.simple());
            let athlete = person(&self.res, &email, name).await;
            join_at(
                &self.res,
                self.group,
                athlete.id,
                athlete.tenant,
                shares,
                Utc::now() - Duration::days(days_ago),
            )
            .await;
            athlete
        }

        async fn roster(&self) -> Value {
            let (status, page) = get(&router(&self.res), ROSTER, &self.coach.auth).await;
            assert_eq!(status, 200, "{page:#}");
            page
        }
    }

    fn reasons(page: &Value, id: Uuid) -> Vec<Value> {
        row(page, id).expect("athlete on the roster")["attention"]
            .as_array()
            .expect("every row carries attention")
            .clone()
    }

    fn kinds(page: &Value, id: Uuid) -> Vec<(String, String)> {
        reasons(page, id)
            .iter()
            .map(|r| {
                (
                    r["kind"].as_str().unwrap().to_owned(),
                    r["horizon"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    fn reason<'a>(list: &'a [Value], kind: &str) -> &'a Value {
        list.iter()
            .find(|r| r["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} in {list:#?}"))
    }

    fn pair(kind: &str, horizon: &str) -> (String, String) {
        (kind.to_owned(), horizon.to_owned())
    }

    /// Every evidence line names a source and a time.
    fn assert_sourced(reason: &Value) {
        for line in reason["evidence"].as_array().unwrap() {
            assert!(line["source"]["kind"].is_string(), "{line:#}");
            assert!(line["key"]
                .as_str()
                .unwrap()
                .starts_with("coachHome.evidence."));
            assert!(
                line["at"].is_string(),
                "every line carries its time: {line:#}"
            );
        }
        assert!(reason["key"]
            .as_str()
            .unwrap()
            .starts_with("coachHome.reason."));
    }

    #[tokio::test]
    async fn a_non_sharer_gets_the_consent_reason_and_nothing_inferred() {
        let s = squad("consent").await;
        let ben = s.athlete("Ben", true, 30).await;
        // Quiet for ten days: a sharer would raise nothing_received.
        seed_ride(&s.res, &ben, 10).await;
        s.res
            .common
            .repos
            .groups
            .update_coach_sharing_consent(&s.group.to_string(), ben.id, false)
            .await
            .unwrap();

        let page = s.roster().await;
        assert_eq!(
            kinds(&page, ben.id),
            [pair("consent_revoked", "today")],
            "the consent reason alone: {page:#}"
        );
        let consent = &reasons(&page, ben.id)[0];
        assert_sourced(consent);
        assert_eq!(consent["action"], "write_to_athlete");
        assert_eq!(
            consent["evidence"][0]["key"],
            "coachHome.evidence.sharingStopped"
        );
        assert_eq!(consent["evidence"][0]["source"]["kind"], "consent");
        let ben_row = row(&page, ben.id).unwrap();
        assert!(
            ben_row.get("season").is_none(),
            "no season for a non-sharer: {ben_row:#}"
        );
        assert!(ben_row.get("training").is_none(), "{ben_row:#}");
        assert!(ben_row["sharing_stopped_at"].is_string(), "{ben_row:#}");
        assert_eq!(consent["evidence"][0]["at"], ben_row["sharing_stopped_at"]);
    }

    #[tokio::test]
    async fn an_athlete_who_never_shared_is_never_read_as_a_stop() {
        let s = squad("never").await;
        // Joined weeks ago without sharing, as the invite let them.
        let hugo = s.athlete("Hugo", false, 30).await;
        // Joined today without sharing: the decision has the join's instant.
        let iris = person(&s.res, "iris@never.test", "Iris").await;
        join(&s.res, s.group, iris.id, iris.tenant, false).await;

        let page = s.roster().await;
        for athlete in [hugo.id, iris.id] {
            let athlete_row = row(&page, athlete).unwrap();
            assert!(
                athlete_row.get("sharing_stopped_at").is_none(),
                "a decline at join is no stop: {athlete_row:#}"
            );
            assert_eq!(
                kinds(&page, athlete),
                Vec::<(String, String)>::new(),
                "never sharing is a standing state, not a reason: {athlete_row:#}"
            );
            assert_eq!(athlete_row["status"], "no_coach_consent");
        }
    }

    #[tokio::test]
    async fn an_agent_reply_with_an_unmatched_figure_is_todays_to_correct() {
        let s = squad("figure").await;
        let felix = s.athlete("Felix", true, 60).await;
        seed_ride(&s.res, &felix, 1).await;
        agent_reply(
            &s.res,
            s.group,
            &felix,
            "Your threshold is 285 W: aim for 240 W on Saturday.",
            &[("your threshold is 285 W", ClaimStatus::Unverifiable)],
        )
        .await;

        let page = s.roster().await;
        assert_eq!(kinds(&page, felix.id), [pair("unverified_figure", "today")]);
        let figure = &reasons(&page, felix.id)[0];
        assert_sourced(figure);
        assert_eq!(figure["action"], "correct_reply");
        assert_eq!(
            figure["evidence"][0]["key"],
            "coachHome.evidence.figureUnmatched"
        );
        assert_eq!(
            figure["evidence"][0]["params"]["figure"],
            "your threshold is 285 W"
        );
        assert_eq!(figure["evidence"][0]["params"]["group"], "Squad");
        assert_eq!(figure["evidence"][0]["source"]["kind"], "verification");
        assert!(figure["target"]["reply_id"].is_string(), "{figure:#}");
        assert_eq!(figure["target"]["group_id"], s.group.to_string());
    }

    #[tokio::test]
    async fn a_dead_source_is_nothing_received_and_reconnect_never_no_activity() {
        let s = squad("dead").await;
        let julien = s.athlete("Julien", true, 60).await;
        seed_ride(&s.res, &julien, 8).await;
        s.res
            .common
            .repos
            .provider_connections
            .mark_needs_reauth(
                julien.id,
                julien.tenant,
                "strava",
                Some("invalid_grant"),
                Utc::now(),
            )
            .await
            .unwrap();

        let page = s.roster().await;
        assert_eq!(
            kinds(&page, julien.id),
            [
                pair("nothing_received", "today"),
                pair("provider_reconnect", "today")
            ],
            "{page:#}"
        );
        let list = reasons(&page, julien.id);
        let quiet = reason(&list, "nothing_received");
        assert_sourced(quiet);
        assert_eq!(quiet["key"], "coachHome.reason.nothingReceivedReconnect");
        assert_eq!(quiet["params"]["provider"], "strava");
        let keys: Vec<&str> = quiet["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["key"].as_str().unwrap())
            .collect();
        assert!(
            keys.contains(&"coachHome.evidence.sourceRefusedSince"),
            "{keys:?}"
        );
        assert!(
            !keys.contains(&"coachHome.evidence.sourceNeedsReconnect"),
            "{keys:?}"
        );
        assert!(
            keys.contains(&"coachHome.evidence.lastReceived"),
            "{keys:?}"
        );
        assert!(
            !keys.contains(&"coachHome.evidence.sourcesConnected"),
            "a dead source never reads as an athlete who stopped: {keys:?}"
        );
        let reconnect = reason(&list, "provider_reconnect");
        assert_sourced(reconnect);
        assert_eq!(reconnect["id"], "provider_reconnect:strava");
        assert_eq!(reconnect["target"]["provider"], "strava");
        let refused = &reconnect["evidence"][0];
        assert_eq!(refused["source"]["kind"], "connection");
        assert_eq!(
            refused["key"], "coachHome.evidence.sourceRefusedSince",
            "the connection recorded when Strava started refusing: {refused:#}"
        );
        assert_eq!(refused["params"]["provider"], "strava");
        assert_eq!(refused["params"]["date"], ymd(Utc::now().date_naive()));
        assert_eq!(
            refused["params"]["date"],
            refused["at"].as_str().unwrap()[..10]
        );
    }

    #[tokio::test]
    async fn quiet_days_on_live_sources_are_this_weeks_by_days() {
        let s = squad("quiet").await;
        let kevin = s.athlete("Kevin", true, 60).await;
        let rosalie = s.athlete("Rosalie", true, 60).await;
        let calm = s.athlete("Calm", true, 60).await;
        seed_ride(&s.res, &kevin, 6).await;
        seed_ride(&s.res, &rosalie, 9).await;
        seed_ride(&s.res, &calm, 2).await;

        let page = s.roster().await;
        assert_eq!(kinds(&page, kevin.id), [pair("nothing_received", "week")]);
        assert_eq!(
            kinds(&page, calm.id),
            Vec::<(String, String)>::new(),
            "a calm athlete"
        );
        let kevin_quiet = &reasons(&page, kevin.id)[0];
        assert_sourced(kevin_quiet);
        assert_eq!(kevin_quiet["key"], "coachHome.reason.nothingReceived");
        assert_eq!(kevin_quiet["params"]["days"], 6);
        assert_eq!(
            kevin_quiet["evidence"].as_array().unwrap().last().unwrap()["key"],
            "coachHome.evidence.sourcesConnected",
            "live sources are said to be live"
        );
        let rosalie_quiet = &reasons(&page, rosalie.id)[0];
        assert!(
            rosalie_quiet["rank"].as_u64() < kevin_quiet["rank"].as_u64(),
            "nine quiet days rank above six"
        );
    }

    /// Fifty steady days, then four very long ones: form ends deep below the
    /// athlete's own fitness for consecutive mornings.
    fn fatigue_block() -> Vec<Activity> {
        let mut rides: Vec<_> = (5..=55)
            .map(|days_ago| ride_lasting(&format!("base-{days_ago}"), days_ago, 3_600))
            .collect();
        rides.extend(
            (1..=4).map(|days_ago| ride_lasting(&format!("block-{days_ago}"), days_ago, 21_600)),
        );
        rides
    }

    #[tokio::test]
    async fn a_run_of_deep_fatigue_mornings_is_todays() {
        let s = squad("fatigue").await;
        let lea = s.athlete("Lea", true, 60).await;
        seed_activities(&s.res, &lea, "strava", &fatigue_block()).await;

        let page = s.roster().await;
        let training = &row(&page, lea.id).unwrap()["training"];
        assert_eq!(
            training["form"]["band"], "deep_fatigue",
            "the block puts today's form in the deep-fatigue band: {training:#}"
        );
        assert_eq!(
            kinds(&page, lea.id),
            [pair("deep_fatigue_streak", "today")],
            "{page:#}"
        );
        let streak = &reasons(&page, lea.id)[0];
        assert_sourced(streak);
        let mornings = streak["params"]["mornings"].as_u64().unwrap();
        assert!(mornings >= 2, "{streak:#}");
        let lines: Vec<&Value> = streak["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["key"] == "coachHome.evidence.formMorning")
            .collect();
        assert_eq!(
            u64::try_from(lines.len()).unwrap(),
            mornings,
            "one line per morning of the run"
        );
        assert_eq!(
            lines[0]["params"]["date"], training["form"]["as_of"],
            "today first"
        );
        assert!(lines
            .iter()
            .all(|l| l["params"]["form_pct"].as_f64().unwrap() < -30.0));
        assert_eq!(lines[0]["source"]["kind"], "training_load");
        assert_eq!(streak["action"], "open_plan_thread");
    }

    #[tokio::test]
    async fn a_draft_waiting_and_a_new_member_are_this_weeks_plan_states() {
        let s = squad("plans").await;
        let mathieu = s.athlete("Mathieu", true, 60).await;
        let camille = s.athlete("Camille", true, 3).await;
        let veteran = s.athlete("Veteran", true, 90).await;
        for athlete in [&mathieu, &camille, &veteran] {
            seed_ride(&s.res, athlete, 1).await;
        }
        // The coach drafts a season for Mathieu, who has none live.
        let today = Utc::now().date_naive();
        let goal = GoalRace {
            name: "Gran Fondo".to_owned(),
            date: ymd(today + Duration::days(120)),
            discipline: "road".to_owned(),
            priority: RacePriority::A,
        };
        let phases = [phase(PhaseKind::Base, today + Duration::days(3), 4)];
        let (tenant_key, athlete_key, coach_key) = (
            mathieu.tenant.to_string(),
            mathieu.id.to_string(),
            s.coach.id.to_string(),
        );
        s.res
            .common
            .repos
            .training_plans
            .save_draft_bundle(&SaveDraftParams {
                owner: DraftOwner {
                    tenant_id: &tenant_key,
                    user_id: &athlete_key,
                    coach_user_id: &coach_key,
                },
                author: PlanAuthor::none(),
                goal_fact_id: None,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "base first",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &[],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();

        let page = s.roster().await;
        assert_eq!(
            kinds(&page, mathieu.id),
            [pair("plan_draft_waiting", "week")]
        );
        let draft = &reasons(&page, mathieu.id)[0];
        assert_sourced(draft);
        assert_eq!(draft["action"], "review_draft");
        assert_eq!(draft["evidence"][0]["source"]["kind"], "plan_thread");
        let mathieu_row = row(&page, mathieu.id).unwrap();
        assert_eq!(
            mathieu_row["season"]["state"], "draft_only",
            "{mathieu_row:#}"
        );
        assert_eq!(
            mathieu_row["plan_thread"]["draft_phases"][0]["kind"], "base",
            "the draft band the season view draws: {mathieu_row:#}"
        );

        assert_eq!(
            kinds(&page, camille.id),
            [pair("new_member_no_plan", "week")]
        );
        let new_member = &reasons(&page, camille.id)[0];
        assert_sourced(new_member);
        assert_eq!(new_member["action"], "create_plan");
        assert_eq!(
            new_member["evidence"][0]["key"],
            "coachHome.evidence.joinedGroup"
        );
        assert_eq!(new_member["evidence"][0]["params"]["group"], "Squad");
        assert_eq!(
            row(&page, camille.id).unwrap()["season"]["state"],
            "new_athlete"
        );

        assert_eq!(
            kinds(&page, veteran.id),
            Vec::<(String, String)>::new(),
            "a long-standing member without a plan raises nothing in the queue"
        );
        assert_eq!(
            row(&page, veteran.id).unwrap()["season"]["state"],
            "no_plan"
        );
    }

    #[tokio::test]
    async fn a_plan_ended_after_its_race_with_nothing_next_is_this_weeks() {
        let s = squad("ended").await;
        let olivier = s.athlete("Olivier", true, 200).await;
        seed_ride(&s.res, &olivier, 1).await;
        let today = Utc::now().date_naive();
        let start = today - Duration::days(7 * 12);
        let race_day = start + Duration::days(7 * 10 - 1);
        let goal = GoalRace {
            name: "Gran Fondo Mont-Tremblant".to_owned(),
            date: ymd(race_day),
            discipline: "road".to_owned(),
            priority: RacePriority::A,
        };
        live_plan(
            &s.res,
            &olivier,
            &goal,
            &[],
            &[
                phase(PhaseKind::Base, start, 6),
                phase(PhaseKind::Build, start + Duration::days(42), 4),
            ],
        )
        .await;

        let page = s.roster().await;
        assert_eq!(
            kinds(&page, olivier.id),
            [pair("plan_ended", "week")],
            "{page:#}"
        );
        let ended = &reasons(&page, olivier.id)[0];
        assert_sourced(ended);
        assert_eq!(ended["params"]["date"], ymd(race_day));
        assert_eq!(
            ended["evidence"][0]["key"],
            "coachHome.evidence.planEndedAfterRace"
        );
        assert_eq!(
            ended["evidence"][0]["params"]["race"],
            "Gran Fondo Mont-Tremblant"
        );
        let season = &row(&page, olivier.id).unwrap()["season"];
        assert_eq!(season["state"], "nothing_since_last_race", "{season:#}");
        assert_eq!(season["race"]["name"], "Gran Fondo Mont-Tremblant");
    }

    #[tokio::test]
    async fn the_plan_carries_its_phases_races_and_an_a_race_ahead() {
        let s = squad("season").await;
        let felix = s.athlete("Felix", true, 60).await;
        seed_ride(&s.res, &felix, 1).await;
        let today = Utc::now().date_naive();
        let start = today - Duration::days(14);
        let goal = GoalRace {
            name: "Ironman Cozumel".to_owned(),
            date: ymd(start + Duration::days(7 * 10 - 1)),
            discipline: "triathlon".to_owned(),
            priority: RacePriority::A,
        };
        let tune_up = GoalRace {
            name: "Half".to_owned(),
            date: ymd(start + Duration::days(7 * 6)),
            discipline: "triathlon".to_owned(),
            priority: RacePriority::B,
        };
        live_plan(
            &s.res,
            &felix,
            &goal,
            slice::from_ref(&tune_up),
            &[
                phase(PhaseKind::Build, start, 4),
                phase(PhaseKind::Specialty, start + Duration::days(28), 4),
                phase(PhaseKind::Taper, start + Duration::days(56), 2),
            ],
        )
        .await;

        let page = s.roster().await;
        let felix_row = row(&page, felix.id).unwrap();
        let plan = &felix_row["plan"];
        assert_eq!(
            plan["phases"],
            json!([
                {"kind": "build", "start": ymd(start), "end": ymd(start + Duration::days(28)), "weeks": 4},
                {"kind": "specialty", "start": ymd(start + Duration::days(28)), "end": ymd(start + Duration::days(56)), "weeks": 4},
                {"kind": "taper", "start": ymd(start + Duration::days(56)), "end": ymd(start + Duration::days(70)), "weeks": 2},
            ]),
            "{plan:#}"
        );
        assert_eq!(
            plan["goal_race"],
            json!({"name": "Ironman Cozumel", "date": goal.date, "priority": "A"})
        );
        assert_eq!(
            plan["races"],
            json!([{"name": "Half", "date": tune_up.date, "priority": "B"}])
        );
        assert_eq!(plan["ends"], ymd(start + Duration::days(70)));
        assert_eq!(
            felix_row["season"],
            json!({
                "state": "a_race_ahead",
                "race": {"name": "Ironman Cozumel", "date": goal.date, "a_race": true},
                "ends": ymd(start + Duration::days(70)),
            })
        );
        assert_eq!(kinds(&page, felix.id), Vec::<(String, String)>::new());
    }

    #[tokio::test]
    async fn the_queue_ranks_today_before_the_week_in_the_decided_order() {
        let s = squad("rank").await;
        let consent = s.athlete("Consent", true, 60).await;
        let figure = s.athlete("Figure", true, 60).await;
        let dead = s.athlete("Dead", true, 60).await;
        let fatigue = s.athlete("Fatigue", true, 60).await;
        let newbie = s.athlete("Newbie", true, 2).await;
        let quiet = s.athlete("Quiet", true, 60).await;
        seed_ride(&s.res, &consent, 1).await;
        seed_ride(&s.res, &figure, 1).await;
        seed_ride(&s.res, &dead, 1).await;
        seed_activities(&s.res, &fatigue, "strava", &fatigue_block()).await;
        seed_ride(&s.res, &newbie, 1).await;
        seed_ride(&s.res, &quiet, 12).await;
        let repos = &s.res.common.repos;
        repos
            .groups
            .update_coach_sharing_consent(&s.group.to_string(), consent.id, false)
            .await
            .unwrap();
        agent_reply(
            &s.res,
            s.group,
            &figure,
            "You rode 412 km last week.",
            &[("you rode 412 km last week", ClaimStatus::Unverifiable)],
        )
        .await;
        repos
            .provider_connections
            .mark_needs_reauth(dead.id, dead.tenant, "strava", None, Utc::now())
            .await
            .unwrap();
        for athlete in [&figure, &dead, &fatigue, &quiet] {
            live_plan(
                &s.res,
                athlete,
                &GoalRace {
                    name: "Race".to_owned(),
                    date: ymd(Utc::now().date_naive() + Duration::days(60)),
                    discipline: "road".to_owned(),
                    priority: RacePriority::A,
                },
                &[],
                &[phase(
                    PhaseKind::Base,
                    Utc::now().date_naive() - Duration::days(7),
                    10,
                )],
            )
            .await;
        }

        let page = s.roster().await;
        let mut queue: Vec<(u64, String)> = page["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|a| a["attention"].as_array().unwrap().clone())
            .map(|r| {
                (
                    r["rank"].as_u64().unwrap(),
                    r["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        queue.sort();
        let order: Vec<&str> = queue.iter().map(|(_, kind)| kind.as_str()).collect();
        assert_eq!(
            order,
            [
                "consent_revoked",
                "unverified_figure",
                "provider_reconnect",
                "deep_fatigue_streak",
                "new_member_no_plan",
                "nothing_received",
            ],
            "{page:#}"
        );
    }

    async fn dismiss(s: &Squad, athlete: Uuid, reason: &Value) -> (u16, Value) {
        post(
            &router(&s.res),
            "/api/coach/attention/dismiss",
            &s.coach.auth,
            &json!({
                "athlete_user_id": athlete,
                "reason_id": reason["id"],
                "fingerprint": reason["fingerprint"],
            }),
        )
        .await
    }

    #[tokio::test]
    async fn a_dismissal_holds_until_the_signal_changes_and_can_be_undone() {
        let s = squad("dismiss").await;
        let kevin = s.athlete("Kevin", true, 60).await;
        seed_ride(&s.res, &kevin, 9).await;

        let before = s.roster().await;
        let quiet = reasons(&before, kevin.id)[0].clone();
        assert_eq!(quiet["dismissed_at"], Value::Null);
        let (status, body) = dismiss(&s, kevin.id, &quiet).await;
        assert_eq!(status, 200, "{body:#}");

        let handled = s.roster().await;
        let still = &reasons(&handled, kevin.id)[0];
        assert_eq!(still["id"], quiet["id"]);
        assert_eq!(
            still["dismissed_at"], body["dismissed_at"],
            "Traité holds: {still:#}"
        );

        // Undo, then dismiss again.
        let (status, restored) = post(
            &router(&s.res),
            "/api/coach/attention/restore",
            &s.coach.auth,
            &json!({"athlete_user_id": kevin.id, "reason_id": quiet["id"]}),
        )
        .await;
        assert_eq!(
            (status, &restored["restored"]),
            (200, &json!(true)),
            "{restored:#}"
        );
        assert_eq!(
            reasons(&s.roster().await, kevin.id)[0]["dismissed_at"],
            Value::Null
        );
        dismiss(&s, kevin.id, &quiet).await;

        // A session arrives, then another quiet spell: a new signal.
        s.res
            .common
            .repos
            .activity_cache
            .upsert_activities(kevin.id, &kevin.tenant, "strava", &[ride("ride-2", 6)])
            .await
            .unwrap();
        let after = s.roster().await;
        let again = &reasons(&after, kevin.id)[0];
        assert_eq!(again["kind"], "nothing_received");
        assert_ne!(again["fingerprint"], quiet["fingerprint"]);
        assert_eq!(
            again["dismissed_at"],
            Value::Null,
            "a changed signal brings the reason back: {again:#}"
        );
    }

    #[tokio::test]
    async fn a_dismissal_is_the_coachs_own_and_stays_in_its_tenant() {
        let s = squad("isolation").await;
        let kevin = s.athlete("Kevin", true, 60).await;
        seed_ride(&s.res, &kevin, 9).await;
        // A second coach holds Kevin too, in another tenant's group.
        let finn = coach(&s.res, "finn@isolation.test", "Finn Coach").await;
        let finn_agent = group_agent(&s.res, &finn).await;
        let finn_group = coached_group(&s.res, finn.tenant, finn.id, &finn_agent, "Finn's").await;
        join(&s.res, finn_group, kevin.id, kevin.tenant, true).await;
        let outsider = person(&s.res, "out@isolation.test", "Out Sider").await;

        let page = s.roster().await;
        let quiet = reasons(&page, kevin.id)[0].clone();
        assert_eq!(dismiss(&s, kevin.id, &quiet).await.0, 200);

        let (_, finn_page) = get(&router(&s.res), ROSTER, &finn.auth).await;
        assert_eq!(
            reasons(&finn_page, kevin.id)[0]["dismissed_at"],
            Value::Null,
            "another coach's queue is untouched: {finn_page:#}"
        );

        // The row lives under Kevin's roster tenant for Casey, and nowhere else.
        let repos = &s.res.common.repos;
        let casey_tenant = s.coach.tenant;
        assert_eq!(
            repos
                .coach_attention_dismissals
                .list_attention_dismissals(casey_tenant, s.coach.id)
                .await
                .unwrap()
                .len(),
            1
        );
        for (tenant, who) in [(finn.tenant, s.coach.id), (casey_tenant, finn.id)] {
            assert!(
                repos
                    .coach_attention_dismissals
                    .list_attention_dismissals(tenant, who)
                    .await
                    .unwrap()
                    .is_empty(),
                "no dismissal leaks across tenant or coach"
            );
        }
        let elsewhere = TenantId::from_uuid(Uuid::new_v4());
        assert!(
            !repos
                .coach_attention_dismissals
                .restore_attention(AttentionDismissalKey {
                    tenant: elsewhere,
                    coach_user_id: s.coach.id,
                    athlete_user_id: kevin.id,
                    reason_id: quiet["id"].as_str().unwrap(),
                })
                .await
                .unwrap(),
            "a restore under another tenant touches nothing"
        );

        // Only an athlete on the caller's roster can be named, and only by a coach.
        let (status, _) = dismiss(&s, outsider.id, &quiet).await;
        assert_eq!(status, 404, "an athlete off the roster");
        let (status, _) = post(
            &router(&s.res),
            "/api/coach/attention/dismiss",
            &outsider.auth,
            &json!({"athlete_user_id": kevin.id, "reason_id": "nothing_received", "fingerprint": "x"}),
        )
        .await;
        assert_eq!(status, 403, "a caller who coaches nothing");
        let (status, _) = post(
            &router(&s.res),
            "/api/coach/attention/dismiss",
            &s.coach.auth,
            &json!({"athlete_user_id": kevin.id, "reason_id": " ", "fingerprint": "x"}),
        )
        .await;
        assert_eq!(status, 400, "an empty reason id");
    }
}
