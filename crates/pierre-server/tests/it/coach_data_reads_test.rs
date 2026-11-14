// ABOUTME: The athlete's record of their human coach's reads: one row per read on every path, none for a refused one
// ABOUTME: The athlete lists only their own rows under their own tenant, /group reads names them, and rows past 12 months are purged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1445. A coach's agent reads a coached athlete's data in the coach's
//! thread, over MCP or A2A with `athlete=`, through the `groups` tools and
//! the plan tools; the coach also reads them outside any tool call — Group
//! info's plan view, `/plan <athlete>`, the REST roster. Each read the
//! coaching gates let through is recorded once, under the athlete's own
//! tenant, and the athlete reads their own record: over REST beside their
//! coach sharing, and with `/group reads`. Rows older than 12 months are
//! purged by the hourly sweep. The coach's thread's own prompt block is
//! pinned in `subject_athlete_stage_test`.

use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Duration, Months, Utc};
use pierre_commands::group_reads::GroupReadsHandler;
use pierre_commands::plan::PlanShowHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::models::{TenantId, TenantPlan, ToolCatalogEntry, ToolCategory};
use pierre_database::repositories::{CoachDataRead, CoachReadClass, NewCoachDataRead};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::coach_read_retention::purge_expired_coach_reads;
use pierre_test_support::delegation::add_group_member;
use pierre_tool_runtime::protocols::UniversalResponse;
use serde_json::{json, Value};
use tower::ServiceExt;
use uuid::Uuid;

use crate::coach_athlete_reads_test::{
    fixture, ok, person, refused_reason, Fixture, Person, ATHLETE_STREAM_RUN, UPLOAD,
};
use crate::common;

/// The athlete's roster name, as a coach names them.
const MEMBER: &str = "Mira";

impl Fixture {
    /// The athlete's recorded reads in the fixture's group, newest first.
    pub(crate) async fn recorded(&self) -> Vec<CoachDataRead> {
        self.repos()
            .coach_data_reads
            .list_coach_reads(self.athlete.tenant, self.athlete.id, self.group, 100)
            .await
            .unwrap()
    }

    /// `who`'s GET of their own record in `group`, with its status.
    async fn get_reads(&self, who: Person, group: Uuid) -> (StatusCode, Value) {
        self.get_as(who, &format!("/api/groups/{group}/members/me/coach-reads"))
            .await
    }

    /// The athlete's recorded reads made through `source`, a tool or a
    /// surface label.
    pub(crate) async fn recorded_from(&self, source: &str) -> Vec<CoachDataRead> {
        self.recorded()
            .await
            .into_iter()
            .filter(|read| read.tool == source)
            .collect()
    }

    /// The athlete stops sharing with their coach.
    async fn stop_sharing(&self) {
        assert!(
            self.repos()
                .groups
                .update_coach_sharing_consent(&self.group.to_string(), self.athlete.id, false)
                .await
                .unwrap(),
            "test setup bug: the athlete's membership row takes the revoked coach sharing"
        );
    }

    /// `who`'s first-party GET of `path`, with its status.
    pub(crate) async fn get_as(&self, who: Person, path: &str) -> (StatusCode, Value) {
        let app = ProviderToolRouter::build_http_app(&self.res);
        // A first-party session, as the app holds it: a token delegated to
        // another application reaches MCP and A2A only.
        let user = self
            .repos()
            .users
            .get_global(who.id)
            .await
            .unwrap()
            .expect("the person has an account");
        let token = common::generate_test_token(&self.res, &user).await;
        let request = Request::get(path)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// One read recorded directly, `ago` before now.
    async fn record_at(&self, tenant: TenantId, athlete: Uuid, ago: Duration) {
        self.repos()
            .coach_data_reads
            .record_coach_read(&NewCoachDataRead {
                tenant,
                athlete,
                coach: self.coach.id,
                group: self.group,
                tool: "get_activities",
                class: CoachReadClass::Training,
                read_at: Utc::now() - ago,
            })
            .await
            .unwrap();
    }
}

// ── A group view's read, once a UTC day ──────────────────────────────────

#[tokio::test]
async fn a_daily_read_is_recorded_once_per_utc_day_and_a_per_read_one_each_time() {
    let f = fixture().await;
    let read_at = Utc::now();
    let read = |read_at| NewCoachDataRead {
        tenant: f.athlete.tenant,
        athlete: f.athlete.id,
        coach: f.coach.id,
        group: f.group,
        tool: "group_stats",
        class: CoachReadClass::Training,
        read_at,
    };
    let reads = &f.repos().coach_data_reads;
    assert!(
        reads.record_daily_coach_read(&read(read_at)).await.unwrap(),
        "bug: the first read of the day is written"
    );
    assert!(
        !reads
            .record_daily_coach_read(&read(read_at + Duration::seconds(1)))
            .await
            .unwrap(),
        "bug: a second read the same UTC day writes nothing, and is no failure"
    );
    assert_eq!(
        f.recorded_from("group_stats").await.len(),
        1,
        "bug: the day's repeat leaves the one row of the day"
    );
    assert!(
        reads
            .record_daily_coach_read(&read(read_at - Duration::days(1)))
            .await
            .unwrap(),
        "bug: a read on another UTC day is written: the key is the day, not the source"
    );
    assert_eq!(
        f.recorded_from("group_stats").await.len(),
        2,
        "bug: another UTC day adds its own row beside the first"
    );

    // The same source recorded per read is never folded into the day's row.
    for _ in 0..2 {
        reads.record_coach_read(&read(read_at)).await.unwrap();
    }
    assert_eq!(
        f.recorded_from("group_stats").await.len(),
        4,
        "bug: a per-read row never conflicts with another"
    );
}

// ── The chokepoint records each read once ───────────────────────────────

#[tokio::test]
async fn a_thread_read_writes_exactly_one_row_naming_the_coach_tool_and_class() {
    let f = fixture().await;
    assert!(
        f.recorded().await.is_empty(),
        "test setup bug: the fixture starts with no record"
    );

    ok(
        f.in_thread("get_activities", json!({})).await,
        "get_activities",
    );
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: one read writes one row: {recorded:?}"
    );
    let row = &recorded[0];
    assert_eq!(row.coach, Some(f.coach.id), "bug: the row names the coach");
    assert_eq!(row.group, f.group, "bug: and the group they coach in");
    assert_eq!(row.tool, "get_activities", "bug: and the tool read through");
    assert_eq!(row.class, CoachReadClass::Training, "bug: and the class");
    assert!(
        (Utc::now() - row.read_at).num_minutes().abs() < 5,
        "bug: and when: {}",
        row.read_at
    );

    ok(
        f.in_thread("get_training_history", json!({})).await,
        "get_training_history",
    );
    assert_eq!(
        f.recorded().await.len(),
        2,
        "bug: a second read writes a second row, and no more"
    );
}

#[tokio::test]
async fn an_mcp_read_with_athlete_writes_one_row() {
    let f = fixture().await;
    ok(
        f.call(
            &f.mcp_executor(),
            f.coach,
            "get_activities",
            json!({ "athlete": MEMBER }),
        )
        .await,
        "get_activities over MCP",
    );
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: one MCP read, one row: {recorded:?}"
    );
    assert_eq!(
        recorded[0].tool, "get_activities",
        "bug: the row names the source the read was made through"
    );
}

#[tokio::test]
async fn a_refused_read_and_a_coachs_own_read_write_none() {
    let f = fixture().await;

    // The coach's own data, in their own conversation: nobody else's read.
    ok(
        f.call(&f.own_executor(), f.coach, "get_activities", json!({}))
            .await,
        "own get_activities",
    );
    // A wellness read the athlete has not granted.
    let refused = f.in_thread("get_sleep_sessions", json!({})).await;
    assert_eq!(
        refused_reason(&refused),
        "wellness_grant_needed",
        "test setup bug: the wellness read is refused for the missing grant"
    );
    // A training read once the athlete stopped sharing.
    f.repos()
        .groups
        .update_coach_sharing_consent(&f.group.to_string(), f.athlete.id, false)
        .await
        .unwrap();
    let refused = f.in_thread("get_activities", json!({})).await;
    assert_eq!(
        refused_reason(&refused),
        "coach_sharing_revoked",
        "test setup bug: the training read is refused for the revoked sharing"
    );

    assert!(
        f.recorded().await.is_empty(),
        "bug: neither a refused read nor the coach's own read is recorded on the athlete"
    );
}

#[tokio::test]
async fn a_read_and_the_call_it_nests_write_one_row_for_the_coachs_call() {
    let f = fixture().await;
    // analyze_activity reads through get_activity_intelligence: a nested
    // dispatch that inherits the athlete. The coach made one call.
    ok(
        f.in_thread(
            "analyze_activity",
            json!({ "activity_id": ATHLETE_STREAM_RUN, "provider": UPLOAD }),
        )
        .await,
        "analyze_activity",
    );
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: the nested call adds no row of its own: {recorded:?}"
    );
    assert_eq!(
        recorded[0].tool, "analyze_activity",
        "bug: the coach's call is named"
    );
}

#[tokio::test]
async fn a_read_repeated_within_one_turn_writes_one_row() {
    let f = fixture().await;
    // One turn: the same executor, so the same turn token. The repeat is
    // answered with a pointer to the result the model already has.
    let turn = f.thread_executor();
    for _ in 0..2 {
        assert!(
            f.call(&turn, f.coach, "get_activities", json!({ "limit": 5 }))
                .await
                .success,
            "test setup bug: both calls answer"
        );
    }
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a repeat answered from the turn's own result reads nothing again"
    );
}

#[tokio::test]
async fn a_read_the_athletes_tenant_switched_off_writes_none() {
    let f = fixture().await;
    let now = Utc::now();
    f.repos()
        .tool_selection
        .upsert_tool_catalog_entry(&ToolCatalogEntry {
            id: Uuid::new_v4().to_string(),
            tool_name: "get_activities".to_owned(),
            display_name: "Get activities".to_owned(),
            description: "test catalog row".to_owned(),
            category: ToolCategory::Fitness,
            is_enabled_by_default: true,
            requires_provider: None,
            min_plan: TenantPlan::Starter,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.res
        .mcp
        .tool_selection
        .set_tool_override(
            f.athlete.tenant,
            "get_activities",
            false,
            f.athlete.id,
            None,
        )
        .await
        .unwrap();

    let refused = f.in_thread("get_activities", json!({})).await;
    assert!(
        !refused.success && refused.result.as_ref().unwrap()["reason"] == "tenant_disabled",
        "test setup bug: the athlete's tenant switched the tool off: {:?}",
        refused.result
    );
    assert!(
        f.recorded().await.is_empty(),
        "bug: a read refused after its subject resolved is not recorded"
    );
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn each_groups_tool_read_of_the_athlete_writes_one_row() {
    use pierre_core::models::ConnectionType;

    let f = fixture().await;
    f.repos()
        .users
        .set_manages_roster(f.coach.id, true)
        .await
        .unwrap();
    f.repos()
        .provider_connections
        .register_connection(
            f.athlete.id,
            f.athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();
    let own = f.own_executor();

    for (tool, args) in [
        ("get_group_member_plan_status", json!({ "member": MEMBER })),
        ("get_roster_overview", json!({})),
        ("get_group_member_activities", json!({ "member": MEMBER })),
    ] {
        let before = f.recorded().await.len();
        // The athlete's Strava holds no token here, so a read may answer
        // with the provider's failure; the gates let it through either way,
        // and the read is recorded before anything of theirs is fetched.
        let _answer = f.try_call(&own, f.coach, tool, args).await;
        let recorded = f.recorded().await;
        assert_eq!(
            recorded.len(),
            before + 1,
            "bug: {tool} records its read of the athlete once: {recorded:?}"
        );
        assert_eq!(recorded[0].tool, tool, "bug: the newest row names {tool}");
    }
}

// ── Every other coach read of the athlete reaches the record ─────────────

/// What a response says, error and result alike.
fn says(response: &UniversalResponse) -> String {
    format!("{:?} {:?}", response.error, response.result)
}

/// The athlete's own season, as they save it themselves.
async fn athlete_season(f: &Fixture) {
    ok(
        f.call(
            &f.own_executor(),
            f.athlete,
            "save_training_plan",
            json!({ "outline": {
                "goal_race": {
                    "name": "Reads Marathon",
                    "date": "2027-05-16",
                    "discipline": "road",
                    "priority": "A",
                },
                "strategy": "build toward the marathon",
            } }),
        )
        .await,
        "the athlete saves their own season",
    );
    assert!(
        f.recorded().await.is_empty(),
        "test setup bug: the athlete's own save is nobody's read"
    );
}

#[tokio::test]
async fn a_plan_tool_acting_for_the_athlete_writes_one_training_row() {
    let f = fixture().await;
    let _answer = f.in_thread("get_training_plan", json!({})).await;
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: the plan read in the coach's thread writes one row: {recorded:?}"
    );
    assert_eq!(
        recorded[0].tool, "get_training_plan",
        "bug: the row names the source the read was made through"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    // Over MCP, with athlete= and no conversation: the same scope.
    let _answer = f
        .call(
            &f.mcp_executor(),
            f.coach,
            "get_training_plan",
            json!({ "athlete": MEMBER }),
        )
        .await;
    assert_eq!(
        f.recorded_from("get_training_plan").await.len(),
        2,
        "bug: the MCP plan read writes its own row, and only one"
    );
}

#[tokio::test]
async fn a_refused_plan_tool_writes_none() {
    let f = fixture().await;
    f.stop_sharing().await;
    let refused = f.in_thread("get_training_plan", json!({})).await;
    assert!(
        says(&refused).contains("stopped sharing"),
        "test setup bug: the plan scope refuses: {refused:?}"
    );
    let refused = f
        .call(
            &f.mcp_executor(),
            f.coach,
            "get_training_plan",
            json!({ "athlete": MEMBER }),
        )
        .await;
    assert!(
        says(&refused).contains("stopped sharing"),
        "test setup bug: the MCP plan scope refuses: {refused:?}"
    );
    assert!(
        f.recorded().await.is_empty(),
        "bug: a refused plan read is not recorded"
    );
}

#[tokio::test]
async fn the_plans_state_adds_one_wellness_row_only_under_the_wellness_grant() {
    let f = fixture().await;
    athlete_season(&f).await;

    let _answer = f
        .in_thread("get_training_plan", json!({ "include_state": true }))
        .await;
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: without the grant the state reads no wellness: {recorded:?}"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    f.share_wellness(true).await;
    let _answer = f
        .in_thread("get_training_plan", json!({ "include_state": true }))
        .await;
    let recorded = f.recorded_from("get_training_plan").await;
    let wellness = recorded
        .iter()
        .filter(|read| read.class == CoachReadClass::Wellness)
        .count();
    assert_eq!(
        (recorded.len(), wellness),
        (3, 1),
        "bug: under the grant the state's read adds one wellness row beside its training \
         row: {recorded:?}"
    );

    // Without include_state the plan read reads no wellness, grant or not.
    let _answer = f.in_thread("get_training_plan", json!({})).await;
    let wellness_after = f
        .recorded_from("get_training_plan")
        .await
        .iter()
        .filter(|read| read.class == CoachReadClass::Wellness)
        .count();
    assert_eq!(wellness_after, 1, "bug: no state, no wellness row");
}

#[tokio::test]
async fn the_plan_view_writes_one_row_and_a_refused_one_none() {
    let f = fixture().await;
    athlete_season(&f).await;
    let path = format!(
        "/api/chat/athletes/{}/plan?group_id={}",
        f.athlete.id, f.group
    );

    let (status, body) = f.get_as(f.coach, &path).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recorded = f.recorded().await;
    assert_eq!(recorded.len(), 1, "bug: one view, one row: {recorded:?}");
    assert_eq!(
        recorded[0].tool, "plan_view",
        "bug: the row names the source the read was made through"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    f.stop_sharing().await;
    let (status, body) = f.get_as(f.coach, &path).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a refused view is not recorded"
    );
}

#[tokio::test]
async fn the_threads_plan_card_read_writes_one_row_and_a_refused_one_none() {
    let f = fixture().await;
    athlete_season(&f).await;
    let path = format!("/api/chat/conversations/{}/subject-plan", f.thread);

    let (status, body) = f.get_as(f.coach, &path).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: one card read, one row: {recorded:?}"
    );
    assert_eq!(
        recorded[0].tool, "coach_thread",
        "bug: the row names the source the read was made through"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    f.stop_sharing().await;
    let (status, body) = f.get_as(f.coach, &path).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a refused card read is not recorded"
    );
}

/// `/plan <query>` as the coach types it in their direct message.
async fn coach_plan_command(f: &Fixture, query: &str) -> String {
    let ctx = PlatformCommandContext {
        user_id: f.coach.id,
        tenant_id: f.coach.tenant,
        channel_type: "telegram".to_owned(),
        args: vec![query.to_owned()],
        raw_text: format!("/plan {query}"),
        ctx: Arc::<ServerContext>::clone(&f.res),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: None,
        conversation_tenant_id: f.coach.tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(&f.res),
    };
    PlanShowHandler.execute(&ctx).await.unwrap().text
}

#[tokio::test]
async fn the_plan_command_writes_one_row_and_a_refused_one_none() {
    let f = fixture().await;
    athlete_season(&f).await;

    let _reply = coach_plan_command(&f, MEMBER).await;
    let recorded = f.recorded().await;
    assert_eq!(recorded.len(), 1, "bug: one /plan, one row: {recorded:?}");
    assert_eq!(
        recorded[0].tool, "plan_command",
        "bug: the row names the source the read was made through"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    f.stop_sharing().await;
    let _refused = coach_plan_command(&f, MEMBER).await;
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a /plan the gates refuse is not recorded"
    );
}

#[tokio::test]
async fn the_rest_roster_writes_one_row_per_sharer_read_and_none_for_a_refused_one() {
    use pierre_core::models::{ConnectionType, Tenant};

    let f = fixture().await;
    f.repos()
        .users
        .set_manages_roster(f.coach.id, true)
        .await
        .unwrap();
    f.repos()
        .provider_connections
        .register_connection(
            f.athlete.id,
            f.athlete.tenant,
            "strava",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let (status, body) = f.get_as(f.coach, "/api/coach/roster").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["athletes"][0]["status"], "shared",
        "test setup bug: the athlete is a sharer: {body}"
    );
    let recorded = f.recorded().await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: one roster read, one row: {recorded:?}"
    );
    assert_eq!(
        recorded[0].tool, "roster",
        "bug: the row names the source the read was made through"
    );
    assert_eq!(
        recorded[0].class,
        CoachReadClass::Training,
        "bug: the row records a read of the athlete's training"
    );

    // The roster still lists them as sharing, but the coach-read gate refuses
    // an athlete of two tenants: nothing of theirs is read, nor recorded.
    let now = Utc::now();
    let second = TenantId::generate();
    f.repos()
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second home".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: f.athlete.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    let (status, body) = f.get_as(f.coach, "/api/coach/roster").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let row = &body["athletes"][0];
    assert!(
        row.get("training").is_none() && row.get("plan").is_none(),
        "bug: a sharer the gate refuses carries no training and no plan: {row}"
    );
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a refused roster read is not recorded"
    );
}

// ── The athlete reads their own record, and only theirs ──────────────────

#[tokio::test]
async fn the_athlete_lists_their_own_reads_and_no_one_elses() {
    let f = fixture().await;
    ok(
        f.in_thread("get_activities", json!({})).await,
        "get_activities",
    );

    let (status, body) = f.get_reads(f.athlete, f.group).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let reads = body["reads"].as_array().expect("a reads array");
    assert_eq!(reads.len(), 1, "{body}");
    assert_eq!(
        reads[0]["coach_name"], "Cole Coach",
        "bug: the read names the coach who made it: {body}"
    );
    assert_eq!(
        reads[0]["tool"], "get_activities",
        "bug: the read names the tool it was made through: {body}"
    );
    assert_eq!(
        reads[0]["data_class"], "training",
        "bug: the read names the kind of data it reached: {body}"
    );
    assert!(reads[0]["read_at"].is_string(), "{body}");
    assert_eq!(body["retention_months"], 12, "{body}");

    // Another member of the same group reads their own record: empty.
    let other = person(&f.res, "other", "Otto Other").await;
    add_group_member(f.repos(), f.group, other.id, other.tenant)
        .await
        .unwrap();
    let (status, body) = f.get_reads(other, f.group).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["reads"].as_array().map(Vec::len),
        Some(0),
        "bug: a member never reads another member's record: {body}"
    );

    // The coach is no member: there is no record of theirs to read here.
    let (status, _) = f.get_reads(f.coach, f.group).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: the coach reads no athlete's record"
    );
}

#[tokio::test]
async fn a_row_under_another_tenant_never_reaches_the_athlete() {
    let f = fixture().await;
    f.record_at(TenantId::generate(), f.athlete.id, Duration::hours(1))
        .await;
    assert!(
        f.recorded().await.is_empty(),
        "bug: the athlete's listing reads their own tenant only"
    );
    let (status, body) = f.get_reads(f.athlete, f.group).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["reads"].as_array().map(Vec::len),
        Some(0),
        "bug: another tenant's row never reaches the athlete: {body}"
    );
}

#[tokio::test]
async fn a_read_under_another_tenant_of_the_athletes_reaches_every_surface() {
    use pierre_core::models::Tenant;

    // A read is recorded under the athlete's home tenant at the time; the
    // athlete may since belong to another, and their session sit in either.
    let f = fixture().await;
    let now = Utc::now();
    let second = TenantId::generate();
    f.repos()
        .tenants
        .create(&Tenant {
            id: second,
            name: "Second home".to_owned(),
            slug: format!("second-{second}"),
            domain: None,
            plan: "professional".to_owned(),
            owner_user_id: f.athlete.id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    f.record_at(f.athlete.tenant, f.athlete.id, Duration::hours(2))
        .await;
    f.record_at(second, f.athlete.id, Duration::hours(1)).await;

    let (status, body) = f.get_reads(f.athlete, f.group).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["reads"].as_array().map(Vec::len),
        Some(2),
        "bug: Group info lists the reads under every tenant the athlete belongs to: {body}"
    );
    let listed = group_reads(&f.res, f.athlete).await;
    assert_eq!(
        listed.matches("training (get_activities)").count(),
        2,
        "bug: /group reads lists the same record as Group info: {listed}"
    );
}

// ── Retention ────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_purge_removes_reads_past_twelve_months_and_keeps_the_rest() {
    let f = fixture().await;
    let twelve_months = Utc::now() - (Utc::now() - Months::new(12));
    f.record_at(
        f.athlete.tenant,
        f.athlete.id,
        twelve_months + Duration::days(2),
    )
    .await;
    f.record_at(
        f.athlete.tenant,
        f.athlete.id,
        twelve_months - Duration::days(2),
    )
    .await;
    f.record_at(f.athlete.tenant, f.athlete.id, Duration::hours(1))
        .await;

    let purged = purge_expired_coach_reads(f.repos().coach_data_reads.as_ref(), Utc::now())
        .await
        .unwrap();
    assert_eq!(purged, 1, "bug: only the read past 12 months goes");
    let kept = f.recorded().await;
    assert_eq!(kept.len(), 2, "bug: the newer reads stay: {kept:?}");
    assert!(
        kept.iter()
            .all(|read| read.read_at > Utc::now() - Months::new(12)),
        "bug: every kept read is within 12 months: {kept:?}"
    );
}

// ── /group reads ─────────────────────────────────────────────────────────

/// The `/group reads` templates, as the catalogue words them in English.
const READS_TEMPLATES: [(&str, &str); 15] = [
    (
        "commands.group.reads_header",
        "Your human coach's reads of your data in {0}, newest first:",
    ),
    ("commands.group.reads_line", "• {0} — {1}: {2} ({3})"),
    (
        "commands.group.reads_none",
        "{0}: your human coach has not read your data in the last {1} months.",
    ),
    ("commands.group.reads_training", "training"),
    ("commands.group.reads_wellness", "wellness"),
    ("commands.group.reads_former_coach", "a former human coach"),
    (
        "commands.group.reads_source_plan_view",
        "your plan, from Group info",
    ),
    (
        "commands.group.reads_source_plan_command",
        "the /plan command",
    ),
    ("commands.group.reads_source_roster", "their roster"),
    (
        "commands.group.reads_source_coach_thread",
        "their thread about you",
    ),
    ("commands.group.reads_source_group_room", "the group's room"),
    (
        "commands.group.reads_source_group_stats",
        "the group's stats",
    ),
    (
        "commands.group.reads_source_group_report",
        "the group's weekly report",
    ),
    (
        "commands.group.reads_source_group_health",
        "the group's health flags",
    ),
    (
        "commands.group.reads_source_weekly_digest",
        "their weekly digest of the group",
    ),
];

/// `/group reads` as `who` types it in their direct message.
async fn group_reads(res: &Arc<ServerContext>, who: Person) -> String {
    let registry = &res.mcp.messaging_strings_registry;
    for (key, template) in READS_TEMPLATES {
        registry.update(key, "en", template.to_owned(), String::new());
    }
    let ctx = PlatformCommandContext {
        user_id: who.id,
        tenant_id: who.tenant,
        channel_type: "telegram".to_owned(),
        args: Vec::new(),
        raw_text: "/group reads".to_owned(),
        ctx: Arc::<ServerContext>::clone(res),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: None,
        conversation_tenant_id: who.tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(res),
    };
    GroupReadsHandler.execute(&ctx).await.unwrap().text
}

#[tokio::test]
async fn group_reads_lists_the_coachs_reads_of_the_member_and_says_when_there_are_none() {
    let f = fixture().await;
    let none = group_reads(&f.res, f.athlete).await;
    assert!(
        none.contains("has not read your data in the last 12 months"),
        "bug: with no read, the command says so: {none}"
    );

    ok(
        f.in_thread("get_activities", json!({})).await,
        "get_activities",
    );
    let listed = group_reads(&f.res, f.athlete).await;
    assert!(
        listed.contains("Reads Squad") && listed.contains("Cole Coach"),
        "bug: the list names the group and the coach: {listed}"
    );
    assert!(
        listed.contains("training (get_activities)"),
        "bug: and the class and the tool: {listed}"
    );

    // A read made outside a tool call reads in words, never as its label.
    let path = format!(
        "/api/chat/athletes/{}/plan?group_id={}",
        f.athlete.id, f.group
    );
    let (status, body) = f.get_as(f.coach, &path).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let listed = group_reads(&f.res, f.athlete).await;
    assert!(
        listed.contains("training (your plan, from Group info)") && !listed.contains("plan_view"),
        "bug: the plan view is named in words: {listed}"
    );
}
