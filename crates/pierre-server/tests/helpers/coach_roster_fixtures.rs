// ABOUTME: Fixtures for the coach roster suites: coaches, athletes, coached groups, cached sessions, agent replies, plans
// ABOUTME: Shared by GET /api/coach/roster and the attention-reason suites so each states only what its case changes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    NewGroupTranscriptEntry, TranscriptSpeaker,
};
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    Activity, ActivityBuilder, AddMessageParams, AgentCategory, ConnectionType, CreateAgentRequest,
    SportType, TenantId,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{
    InsertClaimVerdictParams, PlanOutlineInput, SavePlanBundleParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_attention::CoachAttentionRoutes;
use pierre_mcp_server::routes::coach_roster::CoachRosterRoutes;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_memory::training_plans::{GoalRace, PlanPhase};
use serde_json::Value;
use uuid::Uuid;

use crate::common::{create_test_user_with_plan, generate_test_token};
use crate::helpers::axum_test::AxumTestRequest;

/// A signed-in user.
pub(crate) struct Person {
    pub(crate) id: Uuid,
    pub(crate) tenant: TenantId,
    pub(crate) auth: String,
}

pub(crate) async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
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
pub(crate) async fn coach(res: &Arc<ServerContext>, email: &str, name: &str) -> Person {
    let coach = person(res, email, name).await;
    res.common
        .repos
        .users
        .set_manages_roster(coach.id, true)
        .await
        .unwrap();
    coach
}

pub(crate) async fn group_agent(res: &Arc<ServerContext>, owner: &Person) -> String {
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
pub(crate) async fn coached_group(
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

/// Join `user` to `group` now, sharing with the coach or not.
pub(crate) async fn join(
    res: &Arc<ServerContext>,
    group: Uuid,
    user: Uuid,
    tenant: TenantId,
    shares: bool,
) {
    join_at(res, group, user, tenant, shares, Utc::now()).await;
}

/// Date a membership back to when the case says it began.
const BACKDATE_JOIN_SQL: &str =
    "UPDATE coaching_group_members SET joined_at = $1, coach_sharing_consent_at = $1 \
     WHERE tenant_id = $2 AND group_id = $3 AND user_id = $4";

/// Join `user` to `group` as of `joined_at`: the membership is written now,
/// as every join is, then dated back.
pub(crate) async fn join_at(
    res: &Arc<ServerContext>,
    group: Uuid,
    user: Uuid,
    tenant: TenantId,
    shares: bool,
    joined_at: DateTime<Utc>,
) {
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
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: joined_at,
            joined_at,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
    match res.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(BACKDATE_JOIN_SQL)
                .bind(joined_at)
                .bind(tenant.to_string())
                .bind(group.to_string())
                .bind(user.to_string())
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(BACKDATE_JOIN_SQL)
                .bind(joined_at)
                .bind(tenant.to_string())
                .bind(group)
                .bind(user)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

/// A two-hour 60 km ride `days_ago`.
pub(crate) fn ride(id: &str, days_ago: i64) -> Activity {
    ride_lasting(id, days_ago, 7_200)
}

/// A ride `days_ago` lasting `seconds`, at 30 km/h.
pub(crate) fn ride_lasting(id: &str, days_ago: i64, seconds: u64) -> Activity {
    #[allow(clippy::cast_precision_loss)]
    let metres = seconds as f64 * 30_000.0 / 3_600.0;
    ActivityBuilder::new(
        id.to_owned(),
        format!("ride {id}"),
        SportType::Ride,
        Utc::now() - Duration::days(days_ago),
        seconds,
        "strava".to_owned(),
    )
    .distance_meters(metres)
    .build()
}

/// Connect `provider` for the athlete and warm the cache with `activities`:
/// the snapshot is served from the cache, never a provider.
pub(crate) async fn seed_activities(
    res: &Arc<ServerContext>,
    athlete: &Person,
    provider: &str,
    activities: &[Activity],
) {
    let repos = &res.common.repos;
    repos
        .provider_connections
        .register_connection(
            athlete.id,
            athlete.tenant,
            provider,
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    repos
        .activity_cache
        .upsert_activities(athlete.id, &athlete.tenant, provider, activities)
        .await
        .unwrap();
}

/// A Strava connection and a warm cache of one ride `days_ago`.
pub(crate) async fn seed_ride(res: &Arc<ServerContext>, athlete: &Person, days_ago: i64) {
    seed_activities(res, athlete, "strava", &[ride("ride-1", days_ago)]).await;
}

/// One agent reply to `athlete` in `group`, with the checker's verdicts.
pub(crate) async fn agent_reply(
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

pub(crate) fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// A phase of `kind` opening `start`, `weeks` long.
pub(crate) fn phase(kind: PhaseKind, start: NaiveDate, weeks: u8) -> PlanPhase {
    PlanPhase {
        kind,
        start: ymd(start),
        weeks,
        intent: "the phase's intent".to_owned(),
        target_hours: None,
        purpose: String::new(),
        volume_share_of_peak: None,
        tid_target: None,
        hard_sessions_max: None,
        session_mix: BTreeMap::new(),
        flavour_override: None,
        loading_pattern: None,
        skeleton_id: None,
    }
}

/// Lay `phases` as the athlete's live season toward `goal`, with `races`
/// beside it.
pub(crate) async fn live_plan(
    res: &Arc<ServerContext>,
    athlete: &Person,
    goal: &GoalRace,
    races: &[GoalRace],
    phases: &[PlanPhase],
) {
    let (tenant_key, athlete_key) = (athlete.tenant.to_string(), athlete.id.to_string());
    res.common
        .repos
        .training_plans
        .save_plan_bundle(&SavePlanBundleParams {
            tenant_id: &tenant_key,
            user_id: &athlete_key,
            author: PlanAuthor::none(),
            goal_fact_id: None,
            replace_season: false,
            outline: Some(PlanOutlineInput {
                goal_race: goal,
                races: Some(races),
                strategy: "the season's strategy",
                phases,
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
}

/// The roster read and the attention writes, mounted together.
pub(crate) fn router(res: &Arc<ServerContext>) -> Router {
    CoachRosterRoutes::routes(Arc::clone(res)).merge(CoachAttentionRoutes::routes(Arc::clone(res)))
}

fn parsed(body: String) -> Value {
    serde_json::from_str(&body).unwrap_or(Value::String(body))
}

pub(crate) async fn get(router: &Router, uri: &str, auth: &str) -> (u16, Value) {
    let response = AxumTestRequest::get(uri)
        .header("authorization", auth)
        .send(router.clone())
        .await;
    (response.status(), parsed(response.body_text()))
}

pub(crate) async fn post(router: &Router, uri: &str, auth: &str, body: &Value) -> (u16, Value) {
    let response = AxumTestRequest::post(uri)
        .header("authorization", auth)
        .json(body)
        .send(router.clone())
        .await;
    (response.status(), parsed(response.body_text()))
}

pub(crate) fn row(page: &Value, id: Uuid) -> Option<&Value> {
    page["athletes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["user_id"] == id.to_string())
}
