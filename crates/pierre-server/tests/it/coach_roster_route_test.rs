// ABOUTME: GET /api/coach/roster — the coach's consent-gated roster read for Coach Home (carnet#1251)
// ABOUTME: Consent, revoke, the coaching gate, tenants, pages past 40 athletes, and parity with get_roster_overview
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The roster read hands a coach their athletes' training, so every rule
//! that keeps it to the athletes who share with them is pinned here at the
//! HTTP boundary: no athlete outside their groups, no number for one who does
//! not share, nothing for a caller who coaches nothing — and the same
//! per-athlete reading the agent's `get_roster_overview` sees.

#[cfg(feature = "tools-groups")]
mod coach_roster_route_tests {
    use crate::common::create_test_server_resources;
    use crate::helpers::coach_roster_fixtures::{
        agent_reply, coach, coached_group, get, group_agent, join, person, router, row, seed_ride,
        ymd,
    };

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use chrono::{Datelike, Duration, NaiveDate, Utc};
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::periodization::PhaseKind;
    use pierre_core::transport::TransportPolicy;
    use pierre_database::backends::factory::DatabaseBackend;
    use pierre_database::repositories::training_plans::{
        DraftOwner, PlanAuthor, PublishDraftParams, SaveDraftParams,
    };
    use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SavePlanBundleParams};
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_memory::claims::ClaimStatus;
    use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
    use pierre_tool_runtime::implementations::group_roster::GetRosterOverviewTool;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use uuid::Uuid;

    /// The training-bearing fields a non-sharing row must never carry.
    fn assert_status_only(row: &Value, status: &str) {
        assert_eq!(row["status"], status, "{row:#}");
        for key in ["training", "plan", "plan_thread", "agent_replies"] {
            assert!(
                row.get(key).is_none(),
                "{key} leaked for a {status} row: {row:#}"
            );
        }
    }

    #[tokio::test]
    async fn the_coach_sees_sharers_with_training_and_the_others_by_status_only() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@roster.test", "Casey Coach").await;
        let alex = person(&res, "alex@roster.test", "Alex Athlete").await;
        let ben = person(&res, "ben@roster.test", "Ben Athlete").await;
        let cleo = person(&res, "cleo@roster.test", "Cleo Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        join(&res, group, ben.id, ben.tenant, false).await;
        join(&res, group, cleo.id, cleo.tenant, true).await;
        seed_ride(&res, &alex, 2).await;
        seed_ride(&res, &ben, 1).await;
        // Alex: one reply with an invented figure, two clean ones.
        agent_reply(
            &res,
            group,
            &alex,
            "Your 2:41 marathon in May shows you are ready.",
            &[
                ("your 2:41 marathon in May", ClaimStatus::Unverifiable),
                ("your 21 km long run", ClaimStatus::Supported),
            ],
        )
        .await;
        agent_reply(&res, group, &alex, "Keep Thursday easy.", &[]).await;
        agent_reply(&res, group, &alex, "Good tempo run.", &[]).await;

        let (status, page) = get(&router(&res), "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{page:#}");
        assert_eq!(
            page["scope"],
            json!({"visible_members": 2, "roster_members": 3}),
            "two of the three athletes share with the coach"
        );
        assert_eq!(
            page["groups"][0]["athlete_count"], 3,
            "the group counts every live athlete, sharing or not"
        );

        let alex_row = row(&page, alex.id).expect("Alex is on the roster");
        assert_eq!(alex_row["status"], "shared", "{alex_row:#}");
        assert_eq!(
            alex_row["name"], "Alex Athlete",
            "the row names the athlete by display name"
        );
        assert_eq!(
            alex_row["training"]["days_since_last_activity"], 2,
            "{alex_row:#}"
        );
        assert_eq!(
            alex_row["training"]["last_session"]["sport"], "Ride",
            "{alex_row:#}"
        );
        assert!(
            alex_row["training"].get("tsb").is_none(),
            "no raw TSB on the wire"
        );
        assert_eq!(
            alex_row["agent_replies"],
            json!({"count": 3, "verified": 2, "unmatched_figures": 1, "capped": false}),
            "three replies this week, one carrying an invented figure"
        );
        assert!(
            alex_row.get("plan").is_none(),
            "a sharer with no live plan has no plan summary: {alex_row:#}"
        );
        assert_eq!(
            alex_row["plan_thread"],
            json!({"draft_updated_at": null, "draft_has_conflicts": false, "published_at": null, "draft_phases": []}),
            "a coach who never drafted holds no draft and published nothing"
        );

        // Ben's ride is in the cache, but he does not share: status only.
        assert_status_only(row(&page, ben.id).unwrap(), "no_coach_consent");
        // Cleo shares but has no source.
        assert_status_only(row(&page, cleo.id).unwrap(), "no_source");

        let order: Vec<&str> = page["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            order,
            ["Alex Athlete", "Ben Athlete", "Cleo Athlete"],
            "the sharer leads; the others follow by name"
        );
    }

    #[tokio::test]
    async fn after_a_revoke_the_numbers_disappear() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@revoke.test", "Casey Coach").await;
        let alex = person(&res, "alex@revoke.test", "Alex Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        seed_ride(&res, &alex, 2).await;
        let router = router(&res);

        let (_, before) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert!(
            row(&before, alex.id).unwrap().get("training").is_some(),
            "a sharer's training is listed before the revoke: {before:#}"
        );

        res.common
            .repos
            .groups
            .update_coach_sharing_consent(&group.to_string(), alex.id, false)
            .await
            .unwrap();
        let (status, after) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{after:#}");
        assert_status_only(row(&after, alex.id).unwrap(), "no_coach_consent");
        assert_eq!(
            after["scope"],
            json!({"visible_members": 0, "roster_members": 1}),
            "the revoked athlete stays on the roster but no longer counts as visible"
        );
    }

    #[tokio::test]
    async fn a_caller_who_coaches_nothing_is_refused() {
        let res = create_test_server_resources().await.unwrap();
        let router = router(&res);

        // An athlete: no permission, no group.
        let alex = person(&res, "alex@refuse.test", "Alex Athlete").await;
        let (status, body) = get(&router, "/api/coach/roster", &alex.auth).await;
        assert_eq!(status, 403, "an athlete reads no roster: {body:#}");

        // A roster manager who coaches no group.
        let idle = coach(&res, "idle@refuse.test", "Idle Coach").await;
        let (status, body) = get(&router, "/api/coach/roster", &idle.auth).await;
        assert_eq!(
            status, 403,
            "a roster manager who coaches no group reads no roster: {body:#}"
        );

        // A group's coach whose permission was withdrawn.
        let casey = coach(&res, "casey@refuse.test", "Casey Coach").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        res.common
            .repos
            .users
            .set_manages_roster(casey.id, false)
            .await
            .unwrap();
        let (status, body) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 403, "{body:#}");
        assert!(!body.to_string().contains("Alex"), "{body:#}");
    }

    #[tokio::test]
    async fn tenants_stay_isolated_and_a_group_in_the_athletes_tenant_is_covered() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@tenant.test", "Casey Coach").await;
        let finn = coach(&res, "finn@tenant.test", "Finn Coach").await;
        let alex = person(&res, "alex@tenant.test", "Alex Athlete").await;
        let gus = person(&res, "gus@tenant.test", "Gus Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let finn_agent = group_agent(&res, &finn).await;

        // Casey coaches a group that lives in Alex's tenant (carnet#1210).
        let alex_group = coached_group(&res, alex.tenant, casey.id, &agent, "Alex's crew").await;
        join(&res, alex_group, alex.id, alex.tenant, true).await;
        seed_ride(&res, &alex, 3).await;
        // Finn's athlete, in Finn's tenant.
        let finn_group =
            coached_group(&res, finn.tenant, finn.id, &finn_agent, "Finn's squad").await;
        join(&res, finn_group, gus.id, gus.tenant, true).await;
        seed_ride(&res, &gus, 1).await;
        let router = router(&res);

        let (status, casey_page) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{casey_page:#}");
        assert_eq!(
            row(&casey_page, alex.id).unwrap()["status"],
            "shared",
            "a group in the athlete's tenant is the coach's all the same"
        );
        assert!(
            row(&casey_page, gus.id).is_none(),
            "Finn's athlete leaked to Casey"
        );
        assert_eq!(
            casey_page["scope"]["roster_members"], 1,
            "Casey holds Alex alone"
        );

        let (_, finn_page) = get(&router, "/api/coach/roster", &finn.auth).await;
        assert!(
            row(&finn_page, alex.id).is_none(),
            "Casey's athlete leaked to Finn"
        );
        assert_eq!(
            row(&finn_page, gus.id).unwrap()["status"],
            "shared",
            "Finn sees his own athlete"
        );
    }

    #[tokio::test]
    async fn a_roster_past_forty_athletes_is_served_in_pages() {
        const ATHLETES: usize = 45;
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@pages.test", "Casey Coach").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Big squad").await;
        let mut ids = Vec::with_capacity(ATHLETES);
        for i in 0..ATHLETES {
            let athlete = person(
                &res,
                &format!("a{i}@pages.test"),
                &format!("Athlete {i:02}"),
            )
            .await;
            join(&res, group, athlete.id, athlete.tenant, true).await;
            // A few with a source, read from the cache; the rest have none.
            if i % 15 == 0 {
                seed_ride(&res, &athlete, i64::try_from(i / 15 + 1).unwrap()).await;
            }
            ids.push(athlete.id.to_string());
        }
        let router = router(&res);

        let mut seen = Vec::new();
        let mut offset = Some(0);
        let mut pages = 0;
        while let Some(at) = offset {
            let (status, page) = get(
                &router,
                &format!("/api/coach/roster?offset={at}&limit=20"),
                &casey.auth,
            )
            .await;
            assert_eq!(status, 200, "{page:#}");
            assert_eq!(
                page["scope"]["roster_members"], ATHLETES,
                "every page states the whole roster's scope"
            );
            if at == 0 {
                // The three with a source lead, the longest without a ride
                // first: the order is read off the activity cache.
                let lead: Vec<&str> = page["athletes"].as_array().unwrap()[..3]
                    .iter()
                    .map(|a| a["name"].as_str().unwrap())
                    .collect();
                assert_eq!(lead, ["Athlete 30", "Athlete 15", "Athlete 00"], "{page:#}");
            }
            for athlete in page["athletes"].as_array().unwrap() {
                seen.push(athlete["user_id"].as_str().unwrap().to_owned());
            }
            offset = page["next_offset"]
                .as_u64()
                .map(|n| usize::try_from(n).unwrap());
            pages += 1;
        }
        assert_eq!(pages, 3, "45 athletes at 20 a page is three pages");
        seen.sort();
        ids.sort();
        assert_eq!(seen, ids, "every athlete once, across the pages");
    }

    fn easy_day(date: NaiveDate, workout: &str) -> PlannedDay {
        PlannedDay {
            date: ymd(date),
            sport: "run".to_owned(),
            workout: workout.to_owned(),
            duration_min: Some(45),
            intensity: "Z2".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
        }
    }

    /// The plan summary and the coach's plan thread are read off the
    /// athlete's live season and the coach's own draft, not left empty.
    #[tokio::test]
    async fn a_sharer_with_a_plan_carries_its_summary_and_the_coach_plan_thread() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@plan.test", "Casey Coach").await;
        let alex = person(&res, "alex@plan.test", "Alex Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        seed_ride(&res, &alex, 1).await;

        // A ten-week base phase that opened eight days ago: today is week 2.
        let today = Utc::now().date_naive();
        let season_start = today - Duration::days(8);
        let monday = today - Duration::days(i64::from(today.weekday().num_days_from_monday()));
        let goal = GoalRace {
            name: "Harricana".to_owned(),
            date: ymd(season_start + Duration::days(70)),
            discipline: "trail".to_owned(),
            priority: RacePriority::A,
        };
        let phases = vec![PlanPhase {
            kind: PhaseKind::Base,
            start: ymd(season_start),
            weeks: 10,
            intent: "rebuild volume".to_owned(),
            target_hours: None,
            purpose: String::new(),
            volume_share_of_peak: None,
            tid_target: None,
            hard_sessions_max: None,
            session_mix: BTreeMap::new(),
            flavour_override: None,
            loading_pattern: None,
            skeleton_id: None,
        }];
        let (tenant_key, athlete_key, coach_key) = (
            alex.tenant.to_string(),
            alex.id.to_string(),
            casey.id.to_string(),
        );
        let week_start = ymd(monday);
        let live_days = [easy_day(monday, "Easy 45")];
        let repos = &res.common.repos;
        repos
            .training_plans
            .save_plan_bundle(&SavePlanBundleParams {
                tenant_id: &tenant_key,
                user_id: &athlete_key,
                author: PlanAuthor::none(),
                goal_fact_id: None,
                replace_season: false,
                outline: Some(PlanOutlineInput {
                    goal_race: &goal,
                    races: Some(&[]),
                    strategy: "steady base",
                    phases: &phases,
                    source_conversation_id: None,
                    flavour: None,
                    season_start: None,
                    season_end: None,
                }),
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "base",
                    days: &live_days,
                    adjustment_reason: "",
                    phase_index: None,
                }],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();

        // The coach drafts a change to this week.
        let owner = DraftOwner {
            tenant_id: &tenant_key,
            user_id: &athlete_key,
            coach_user_id: &coach_key,
        };
        let draft_days = [easy_day(monday, "Steady 50")];
        repos
            .training_plans
            .save_draft_bundle(&SaveDraftParams {
                owner,
                author: PlanAuthor::none(),
                goal_fact_id: None,
                outline: None,
                weeks: &[PlanWeekInput {
                    week_start: &week_start,
                    focus: "base",
                    days: &draft_days,
                    adjustment_reason: "coach tweak",
                    phase_index: None,
                }],
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
        let router = router(&res);

        let (status, drafted) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{drafted:#}");
        let alex_row = row(&drafted, alex.id).expect("Alex is on the roster");
        let plan = &alex_row["plan"];
        assert_eq!(
            plan["phase"], "base",
            "today falls in the base phase: {plan:#}"
        );
        assert_eq!(plan["week"], 2, "eight days in is week 2: {plan:#}");
        assert_eq!(
            plan["weeks_total"], 10,
            "the phases span ten weeks: {plan:#}"
        );
        assert_eq!(
            plan["goal_race"],
            json!({"name": "Harricana", "date": goal.date, "priority": "A"}),
            "the goal race is the season's A race"
        );
        assert!(
            alex_row["plan_thread"]["draft_updated_at"].is_string(),
            "the coach's open draft is reported: {alex_row:#}"
        );
        assert_eq!(
            alex_row["plan_thread"]["published_at"],
            Value::Null,
            "nothing is published yet: {alex_row:#}"
        );

        repos
            .training_plans
            .publish_draft(&PublishDraftParams {
                owner,
                overwrite: false,
                goal_fact_id: None,
            })
            .await
            .unwrap();
        let (_, published) = get(&router, "/api/coach/roster", &casey.auth).await;
        let thread = &row(&published, alex.id).unwrap()["plan_thread"];
        assert!(
            thread["published_at"].is_string(),
            "the publish is stamped on the coach's plan thread: {thread:#}"
        );
        assert_eq!(
            thread["draft_updated_at"],
            Value::Null,
            "a published draft is no longer open: {thread:#}"
        );
    }

    /// The agent tool and the REST read project the same athlete the same way.
    #[tokio::test]
    async fn the_tool_and_the_route_read_the_same_athlete_the_same_way() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@parity.test", "Casey Coach").await;
        let alex = person(&res, "alex@parity.test", "Alex Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        seed_ride(&res, &alex, 4).await;

        let (_, page) = get(&router(&res), "/api/coach/roster", &casey.auth).await;
        let rest = &row(&page, alex.id).unwrap()["training"];

        let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&res);
        let ctx = ToolContext::new()
            .with_user(casey.id.to_string())
            .with_tenant(casey.tenant.to_string())
            .with_auth_method("jwt_bearer");
        let tool = GetRosterOverviewTool
            .execute(&runtime, &ctx, json!({}))
            .await
            .structured_content
            .expect("structured content");
        let agent_row = tool["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == "Alex Athlete")
            .expect("Alex in the tool's roster");
        let tool_training = &agent_row["training"];

        for key in [
            "ctl",
            "days_since_last_activity",
            "last_activity_per_source",
            "needs_reconnect",
            "stale",
        ] {
            assert_eq!(
                rest[key], tool_training[key],
                "{key} differs: {rest:#} vs {tool_training:#}"
            );
        }
        assert_eq!(
            rest["last_session"]["date"], tool_training["recent_activities"][0]["date"],
            "the last session is the tool's most recent activity"
        );
        assert_eq!(
            page["scope"]["visible_members"], 1,
            "Alex is the one visible athlete"
        );
    }

    /// Forget when a membership's coach sharing was last decided, as every
    /// membership from before the instant was recorded has it.
    async fn forget_sharing_instant(res: &Arc<ServerContext>, group: Uuid, user: Uuid) {
        const SQL: &str = "UPDATE coaching_group_members SET coach_sharing_consent_at = NULL \
                           WHERE group_id = $1 AND user_id = $2";
        match res.agent.database.backend() {
            DatabaseBackend::SQLite(db) => {
                sqlx::query(SQL)
                    .bind(group.to_string())
                    .bind(user.to_string())
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(db) => {
                sqlx::query(SQL)
                    .bind(group)
                    .bind(user)
                    .execute(db.pool())
                    .await
                    .unwrap();
            }
        }
    }

    #[tokio::test]
    async fn a_revoke_names_the_day_and_a_regrant_clears_it() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@stopped.test", "Casey Coach").await;
        let alex = person(&res, "alex@stopped.test", "Alex Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, true).await;
        seed_ride(&res, &alex, 2).await;
        let router = router(&res);
        let groups = &res.common.repos.groups;

        let joined = groups
            .get_member(&group.to_string(), alex.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            joined.coach_sharing_consent_at.is_some(),
            "joining grants coach sharing and stamps the instant"
        );

        let before_revoke = Utc::now();
        groups
            .update_coach_sharing_consent(&group.to_string(), alex.id, false)
            .await
            .unwrap();
        let revoked = groups
            .get_member(&group.to_string(), alex.id)
            .await
            .unwrap()
            .unwrap();
        let revoked_at = revoked.coach_sharing_consent_at.unwrap();
        assert!(
            revoked_at >= before_revoke,
            "the revoke stamps its own instant"
        );

        let (status, page) = get(&router, "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{page:#}");
        let alex_row = row(&page, alex.id).unwrap();
        assert_status_only(alex_row, "no_coach_consent");
        let stopped =
            chrono::DateTime::parse_from_rfc3339(alex_row["sharing_stopped_at"].as_str().unwrap())
                .unwrap()
                .with_timezone(&Utc);
        assert_eq!(
            stopped.date_naive(),
            revoked_at.date_naive(),
            "the roster names the revoke's day: {alex_row:#}"
        );

        let before_regrant = Utc::now();
        groups
            .update_coach_sharing_consent(&group.to_string(), alex.id, true)
            .await
            .unwrap();
        let regranted = groups
            .get_member(&group.to_string(), alex.id)
            .await
            .unwrap()
            .unwrap();
        let regranted_at = regranted.coach_sharing_consent_at.unwrap();
        assert!(
            regranted_at > revoked_at,
            "a re-grant moves the stored instant past the revoke"
        );
        assert!(
            regranted_at >= before_regrant,
            "the re-grant stamps its own instant"
        );
        let (_, page) = get(&router, "/api/coach/roster", &casey.auth).await;
        let alex_row = row(&page, alex.id).unwrap();
        assert_eq!(alex_row["status"], "shared", "{alex_row:#}");
        assert!(
            alex_row.get("sharing_stopped_at").is_none(),
            "a sharer carries no stop date: {alex_row:#}"
        );
    }

    /// The instant the roster names for `user`, parsed, or `None` when absent.
    fn stopped_at(page: &Value, user: Uuid) -> Option<chrono::DateTime<Utc>> {
        row(page, user)?.get("sharing_stopped_at").map(|v| {
            chrono::DateTime::parse_from_rfc3339(v.as_str().unwrap())
                .unwrap()
                .with_timezone(&Utc)
        })
    }

    async fn stamp_of(
        res: &Arc<ServerContext>,
        group: Uuid,
        user: Uuid,
    ) -> Option<chrono::DateTime<Utc>> {
        res.common
            .repos
            .groups
            .get_member(&group.to_string(), user)
            .await
            .unwrap()
            .unwrap()
            .coach_sharing_consent_at
    }

    #[tokio::test]
    async fn the_stop_date_merges_across_the_coachs_groups() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@merge.test", "Casey Coach").await;
        let agent = group_agent(&res, &casey).await;
        let a = coached_group(&res, casey.tenant, casey.id, &agent, "Squad A").await;
        let b = coached_group(&res, casey.tenant, casey.id, &agent, "Squad B").await;
        let groups = &res.common.repos.groups;
        let revoke = |g: Uuid, u: Uuid| async move {
            groups
                .update_coach_sharing_consent(&g.to_string(), u, false)
                .await
                .unwrap();
        };

        // Revoked in A, still sharing in B, and the other way round: shared.
        let a_then_b = person(&res, "ab@merge.test", "Ab").await;
        join(&res, a, a_then_b.id, a_then_b.tenant, false).await;
        join(&res, b, a_then_b.id, a_then_b.tenant, true).await;
        revoke(a, a_then_b.id).await;
        let b_then_a = person(&res, "ba@merge.test", "Ba").await;
        join(&res, a, b_then_a.id, b_then_a.tenant, true).await;
        join(&res, b, b_then_a.id, b_then_a.tenant, false).await;
        revoke(b, b_then_a.id).await;
        seed_ride(&res, &a_then_b, 2).await;
        seed_ride(&res, &b_then_a, 2).await;

        // Revoked in both, B later: the later instant wins.
        let both = person(&res, "both@merge.test", "Both").await;
        join(&res, a, both.id, both.tenant, true).await;
        join(&res, b, both.id, both.tenant, true).await;
        revoke(a, both.id).await;
        revoke(b, both.id).await;

        // Revoked in both, one instant unknown: the dated one, in either order.
        let a_unknown = person(&res, "au@merge.test", "Au").await;
        let b_unknown = person(&res, "bu@merge.test", "Bu").await;
        for (user, unknown) in [(&a_unknown, a), (&b_unknown, b)] {
            join(&res, a, user.id, user.tenant, true).await;
            join(&res, b, user.id, user.tenant, true).await;
            revoke(a, user.id).await;
            revoke(b, user.id).await;
            forget_sharing_instant(&res, unknown, user.id).await;
        }

        // Revoked in both, neither instant known: no date at all.
        let neither = person(&res, "none@merge.test", "None").await;
        join(&res, a, neither.id, neither.tenant, true).await;
        join(&res, b, neither.id, neither.tenant, true).await;
        revoke(a, neither.id).await;
        revoke(b, neither.id).await;
        forget_sharing_instant(&res, a, neither.id).await;
        forget_sharing_instant(&res, b, neither.id).await;

        let (status, page) = get(&router(&res), "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{page:#}");

        for shared in [&a_then_b, &b_then_a] {
            assert_eq!(
                row(&page, shared.id).unwrap()["status"],
                "shared",
                "{page:#}"
            );
            assert!(
                stopped_at(&page, shared.id).is_none(),
                "one group still shares, so there is no stop date: {page:#}"
            );
        }
        let both_a = stamp_of(&res, a, both.id).await.unwrap();
        let both_b = stamp_of(&res, b, both.id).await.unwrap();
        assert!(both_b > both_a, "the fixture revoked B after A");
        assert_eq!(
            stopped_at(&page, both.id),
            Some(both_b),
            "the latest revoke is the stop date: {page:#}"
        );
        assert_eq!(
            stopped_at(&page, a_unknown.id),
            stamp_of(&res, b, a_unknown.id).await,
            "A's instant is unknown, B's is named: {page:#}"
        );
        assert_eq!(
            stamp_of(&res, a, a_unknown.id).await,
            None,
            "the dated revoke beats the unknown one: {page:#}"
        );
        assert_eq!(
            stopped_at(&page, b_unknown.id),
            stamp_of(&res, a, b_unknown.id).await,
            "the dated revoke beats the unknown one: {page:#}"
        );
        assert!(
            stopped_at(&page, a_unknown.id).is_some() && stopped_at(&page, b_unknown.id).is_some(),
            "a known instant is named: {page:#}"
        );
        let neither_row = row(&page, neither.id).unwrap();
        assert_status_only(neither_row, "no_coach_consent");
        assert!(
            neither_row.get("sharing_stopped_at").is_none(),
            "no known instant in any group, so no date: {neither_row:#}"
        );
    }

    #[tokio::test]
    async fn a_revoke_from_before_the_instant_was_recorded_carries_no_date() {
        let res = create_test_server_resources().await.unwrap();
        let casey = coach(&res, "casey@legacy.test", "Casey Coach").await;
        let alex = person(&res, "alex@legacy.test", "Alex Athlete").await;
        let agent = group_agent(&res, &casey).await;
        let group = coached_group(&res, casey.tenant, casey.id, &agent, "Squad").await;
        join(&res, group, alex.id, alex.tenant, false).await;
        forget_sharing_instant(&res, group, alex.id).await;
        let legacy = res
            .common
            .repos
            .groups
            .get_member(&group.to_string(), alex.id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            legacy.coach_sharing_consent_at.is_none(),
            "the legacy fixture really has no instant"
        );

        let (status, page) = get(&router(&res), "/api/coach/roster", &casey.auth).await;
        assert_eq!(status, 200, "{page:#}");
        let alex_row = row(&page, alex.id).unwrap();
        assert_status_only(alex_row, "no_coach_consent");
        assert!(
            alex_row.get("sharing_stopped_at").is_none(),
            "an unknown date is absent, never guessed: {alex_row:#}"
        );
    }
}
