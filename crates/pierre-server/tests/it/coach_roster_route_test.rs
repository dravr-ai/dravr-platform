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
    use crate::common::{
        create_test_server_resources, create_test_user_with_plan, generate_test_token,
    };
    use crate::helpers::axum_test::AxumTestRequest;

    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axum::Router;
    use chrono::{Datelike, Duration, NaiveDate, Utc};
    use dravr_tronc::mcp::tool::{McpTool, ToolContext};
    use pierre_core::models::groups::{
        CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
        NewGroupTranscriptEntry, TranscriptSpeaker,
    };
    use pierre_core::models::periodization::PhaseKind;
    use pierre_core::models::{
        Activity, ActivityBuilder, AddMessageParams, AgentCategory, ConnectionType,
        CreateAgentRequest, SportType, TenantId,
    };
    use pierre_core::transport::TransportPolicy;
    use pierre_database::repositories::training_plans::{
        DraftOwner, PlanAuthor, PublishDraftParams, SaveDraftParams,
    };
    use pierre_database::repositories::{
        InsertClaimVerdictParams, PlanOutlineInput, PlanWeekInput, SavePlanBundleParams,
    };
    use pierre_mcp_server::mcp::resources::ServerContext;
    use pierre_mcp_server::routes::coach_roster::CoachRosterRoutes;
    use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
    use pierre_memory::training_plans::{GoalRace, PlanPhase, PlannedDay, RacePriority};
    use pierre_tool_runtime::implementations::group_roster::GetRosterOverviewTool;
    use pierre_tool_runtime::runtime::ToolRuntime;
    use serde_json::{json, Value};
    use uuid::Uuid;

    struct Person {
        id: Uuid,
        tenant: TenantId,
        auth: String,
    }

    async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
        let (id, user, tenant) =
            create_test_user_with_plan(&res.agent.database, email, "professional")
                .await
                .unwrap();
        res.common
            .repos
            .users
            .update_display_name(id, name)
            .await
            .unwrap();
        let auth = format!("Bearer {}", generate_test_token(res, &user).await);
        Person { id, tenant, auth }
    }

    /// A coach holds `manages_roster`, as attaching them to a group requires.
    async fn coach(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
        let coach = person(res, email, name).await;
        res.common
            .repos
            .users
            .set_manages_roster(coach.id, true)
            .await
            .unwrap();
        coach
    }

    async fn group_agent(res: &Arc<ServerContext>, owner: &Person) -> String {
        res.common
            .repos
            .agents
            .create(
                owner.id,
                owner.tenant,
                &CreateAgentRequest {
                    title: "Group Agent".to_owned(),
                    description: None,
                    system_prompt: "You are helpful".to_owned(),
                    category: AgentCategory::Custom,
                    tags: vec![],
                    sample_prompts: vec![],
                    startup_query: None,
                    data_requirements: None,
                    purpose: None,
                    when_to_use: None,
                    instructions: None,
                    example_inputs: None,
                    example_outputs: None,
                    success_criteria: None,
                    max_tool_iterations: None,
                },
            )
            .await
            .unwrap()
            .id
            .to_string()
    }

    /// A group in `tenant` whose human coach is `coach`.
    async fn coached_group(
        res: &Arc<ServerContext>,
        tenant: TenantId,
        coach: Uuid,
        agent_id: &str,
        name: &str,
    ) -> Uuid {
        let now = Utc::now();
        res.common
            .repos
            .groups
            .create_group(
                tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: tenant.to_string(),
                    name: name.to_owned(),
                    description: None,
                    agent_id: agent_id.to_owned(),
                    owner_id: coach,
                    coach_user_id: Some(coach),
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 100,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn join(
        res: &Arc<ServerContext>,
        group: Uuid,
        user: Uuid,
        tenant: TenantId,
        shares: bool,
    ) {
        let now = Utc::now();
        res.common
            .repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group,
                user_id: user,
                tenant_id: tenant.to_string(),
                role: GroupRole::Member,
                peer_sharing_consent: false,
                coach_sharing_consent: shares,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    fn ride(id: &str, days_ago: i64) -> Activity {
        ActivityBuilder::new(
            id.to_owned(),
            format!("ride {id}"),
            SportType::Ride,
            Utc::now() - Duration::days(days_ago),
            7_200,
            "strava".to_owned(),
        )
        .distance_meters(60_000.0)
        .build()
    }

    /// A Strava connection and a warm cache of one ride `days_ago`: the
    /// snapshot is served from the cache, never a provider.
    async fn seed_ride(res: &Arc<ServerContext>, athlete: &Person, days_ago: i64) {
        let repos = &res.common.repos;
        repos
            .provider_connections
            .register_connection(
                athlete.id,
                athlete.tenant,
                "strava",
                &ConnectionType::OAuth,
                None,
            )
            .await
            .unwrap();
        repos
            .activity_cache
            .upsert_activities(
                athlete.id,
                &athlete.tenant,
                "strava",
                &[ride("ride-1", days_ago)],
            )
            .await
            .unwrap();
    }

    /// One agent reply to `athlete` in `group`, with the checker's verdicts.
    async fn agent_reply(
        res: &Arc<ServerContext>,
        group: Uuid,
        athlete: &Person,
        content: &str,
        verdicts: &[(&str, ClaimStatus)],
    ) {
        let repos = &res.common.repos;
        let user = athlete.id.to_string();
        let conversation = repos
            .chat
            .create_conversation(
                &user,
                athlete.tenant,
                "Group",
                "model",
                None,
                Some(&group.to_string()),
            )
            .await
            .unwrap();
        let message = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id: athlete.tenant,
                conversation_id: &conversation.id,
                user_id: &user,
                role: "assistant",
                content,
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await
            .unwrap();
        repos
            .groups
            .append_transcript_entry(&NewGroupTranscriptEntry {
                group_id: &group.to_string(),
                tenant_id: &athlete.tenant.to_string(),
                author_user_id: athlete.id,
                speaker: TranscriptSpeaker::Coach,
                content,
                source_conversation_id: Some(&conversation.id),
                source_message_id: Some(&message.id),
                transport_policy: TransportPolicy::AnyTransport,
            })
            .await
            .unwrap();
        for (claim, status) in verdicts {
            repos
                .claim_verdicts
                .insert_claim_verdict(&InsertClaimVerdictParams {
                    tenant_id: athlete.tenant,
                    user_id: &user,
                    agent_id: None,
                    conversation_id: None,
                    message_id: Some(&message.id),
                    claim_text: claim,
                    category: ClaimCategory::AthleteData,
                    status: *status,
                    evidence_strength: EvidenceStrength::None,
                    confidence: 0.0,
                    layer_fired: VerdictLayer::AthleteData,
                    explanation: None,
                    evidence_refs: None,
                    transport_policy: TransportPolicy::AnyTransport,
                })
                .await
                .unwrap();
        }
    }

    fn router(res: &Arc<ServerContext>) -> Router {
        CoachRosterRoutes::routes(Arc::clone(res))
    }

    async fn get(router: &Router, uri: &str, auth: &str) -> (u16, Value) {
        let response = AxumTestRequest::get(uri)
            .header("authorization", auth)
            .send(router.clone())
            .await;
        let status = response.status();
        let body = response.body_text();
        (
            status,
            serde_json::from_str(&body).unwrap_or(Value::String(body)),
        )
    }

    fn row(page: &Value, id: Uuid) -> Option<&Value> {
        page["athletes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["user_id"] == id.to_string())
    }

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
            json!({"draft_updated_at": null, "draft_has_conflicts": false, "published_at": null}),
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

    fn ymd(date: NaiveDate) -> String {
        date.format("%Y-%m-%d").to_string()
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
            fuel_product_ids: Vec::new(),
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
            json!({"name": "Harricana", "date": goal.date}),
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
}
