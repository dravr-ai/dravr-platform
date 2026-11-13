// ABOUTME: The group views a human coach opens over every member's training reach each member's read log, once a day
// ABOUTME: Room cards, /stats, /report and /health ask the coach-read gate first; a training tool's wellness read adds its own row

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1445. A group's human coach reads every member's training at once
//! in four places this suite drives: the member cards their agent reads on a
//! turn in the group's room, and the group's stats, weekly report and health
//! flags. Each member those views read goes on the member's own record, at
//! most once a UTC day per view, and only once the member passes the one
//! coach-read gate every other coach read asks (carnet#1425). The weekly
//! digest's coach copy is pinned in `group_digest_room_test`.
//!
//! A training tool that also reads the athlete's wellness under their grant
//! puts a wellness row beside its training row; without the grant it reads
//! none and records none.

use chrono::Utc;
#[cfg(feature = "tools-groups")]
use pierre_chat_pipeline::stages::prompt_builder::resolve_group_context;
use pierre_core::models::groups::{CoachConsents, GroupMember, GroupRole};
use pierre_core::models::{Tenant, TenantId};
use pierre_database::repositories::CoachReadClass;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::coach_athlete_reads_test::{fixture, ok, Fixture, ATHLETE_STREAM_RUN, UPLOAD};
use crate::coach_athlete_wellness_reads_test::world;

impl Fixture {
    /// The athlete belongs to a second tenant too: the coach-read gate's
    /// home-tenant check refuses them, though they still share with the coach.
    async fn athlete_joins_a_second_tenant(&self) {
        let now = Utc::now();
        let second = TenantId::generate();
        self.repos()
            .tenants
            .create(&Tenant {
                id: second,
                name: "Second home".to_owned(),
                slug: format!("second-{second}"),
                domain: None,
                plan: "professional".to_owned(),
                owner_user_id: self.athlete.id,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }

    /// The coach also manages the group, as its owner: the analytics answer
    /// a manager only.
    async fn coach_manages_the_group(&self) {
        let now = Utc::now();
        self.repos()
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: self.group,
                user_id: self.coach.id,
                tenant_id: self.coach.tenant.to_string(),
                role: GroupRole::Owner,
                peer_sharing_consent: true,
                coach_sharing_consent: true,
                coach_sharing_consent_at: None,
                coach_consents: CoachConsents::default(),
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }

    /// The coach's GET of one of the group's analytics, which must answer.
    async fn analytics(&self, view: &str) -> Value {
        let (status, body) = self
            .get_as(self.coach, &format!("/api/groups/{}/{view}", self.group))
            .await;
        assert!(status.is_success(), "{view}: {status} {body}");
        body
    }

    /// The member cards the coach's agent reads on a turn in the group's
    /// room: the members whose snapshot the turn fetched.
    #[cfg(feature = "tools-groups")]
    async fn room_cards(&self) -> Vec<Uuid> {
        let ctx = self.res.chat_pipeline_context();
        let group = self.group.to_string();
        let (_, snapshots) = resolve_group_context(
            &ctx,
            Some(group.as_str()),
            self.coach.tenant,
            self.coach.id,
            Some(self.coach.id),
        )
        .await
        .unwrap();
        snapshots.into_iter().map(|s| s.user_id).collect()
    }
}

// ── The group views ──────────────────────────────────────────────────────

#[tokio::test]
async fn each_analytics_view_records_the_member_once_a_day() {
    let f = fixture().await;
    f.coach_manages_the_group().await;
    for (view, source) in [
        ("stats", "group_stats"),
        ("report", "group_report"),
        ("health", "group_health"),
    ] {
        let first = f.analytics(view).await;
        assert_eq!(
            first["scope"]["visible_members"], 2,
            "test setup bug: the coach and the athlete are in the view: {first}"
        );
        let recorded = f.recorded_from(source).await;
        assert_eq!(
            recorded.len(),
            1,
            "bug: the {view} view puts the athlete's read on their record: {recorded:?}"
        );
        let row = &recorded[0];
        assert_eq!(row.coach, Some(f.coach.id), "bug: the row names the coach");
        assert_eq!(row.class, CoachReadClass::Training, "bug: a training read");

        let _again = f.analytics(view).await;
        assert_eq!(
            f.recorded_from(source).await.len(),
            1,
            "bug: a second {view} view the same day adds no row"
        );
    }
    assert_eq!(
        f.recorded().await.len(),
        3,
        "bug: one row per view, and none for the coach's own training"
    );
}

#[tokio::test]
async fn an_athlete_the_gate_refuses_is_left_out_of_the_analytics_and_off_the_record() {
    let f = fixture().await;
    f.coach_manages_the_group().await;
    f.athlete_joins_a_second_tenant().await;

    for view in ["stats", "report", "health"] {
        let body = f.analytics(view).await;
        assert_eq!(
            body["scope"]["visible_members"], 1,
            "bug: an athlete the coach-read gate refuses is not read for the {view} view, \
             though they share with the coach: {body}"
        );
    }
    assert!(
        f.recorded().await.is_empty(),
        "bug: a refused read is not recorded"
    );
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn the_room_cards_record_the_member_once_a_day() {
    let f = fixture().await;
    assert_eq!(
        f.room_cards().await,
        vec![f.athlete.id],
        "test setup bug: the coach's room turn reads the athlete's card, not the coach's"
    );
    let recorded = f.recorded_from("group_room").await;
    assert_eq!(
        recorded.len(),
        1,
        "bug: the room's cards put the athlete's read on their record: {recorded:?}"
    );
    assert_eq!(
        recorded[0].coach,
        Some(f.coach.id),
        "bug: the row names the coach whose turn read the cards"
    );

    assert_eq!(
        f.room_cards().await,
        vec![f.athlete.id],
        "bug: the second turn the same day still reads the athlete's card"
    );
    assert_eq!(
        f.recorded().await.len(),
        1,
        "bug: a second room turn the same day adds no row"
    );
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn an_athlete_the_gate_refuses_gets_no_room_card_and_no_row() {
    let f = fixture().await;
    f.athlete_joins_a_second_tenant().await;
    assert!(
        f.room_cards().await.is_empty(),
        "bug: nothing of an athlete the coach-read gate refuses is read for the coach's room"
    );
    assert!(
        f.recorded().await.is_empty(),
        "bug: a refused read is not recorded"
    );
}

#[cfg(feature = "tools-groups")]
#[tokio::test]
async fn a_member_reads_the_group_views_of_their_peers_with_nothing_recorded() {
    let f = fixture().await;
    // The athlete's own turn in the room reads their peers' sharing, which is
    // no coach read.
    let ctx = f.res.chat_pipeline_context();
    let group = f.group.to_string();
    let (_, snapshots) = resolve_group_context(
        &ctx,
        Some(group.as_str()),
        f.coach.tenant,
        f.athlete.id,
        None,
    )
    .await
    .unwrap();
    assert!(
        snapshots.iter().any(|s| s.user_id == f.athlete.id),
        "test setup bug: the athlete's own card is read"
    );
    assert!(
        f.recorded().await.is_empty(),
        "bug: a member's own view of the room is no coach read"
    );
}

// ── A training tool's wellness read ──────────────────────────────────────

/// The athlete's recorded reads through `tool`, as (training, wellness).
async fn classes(f: &Fixture, tool: &str) -> (usize, usize) {
    let recorded = f.recorded_from(tool).await;
    let wellness = recorded
        .iter()
        .filter(|read| read.class == CoachReadClass::Wellness)
        .count();
    (recorded.len() - wellness, wellness)
}

#[tokio::test]
async fn a_training_tool_that_reads_wellness_records_both_rows_only_under_the_grant() {
    use pierre_core::constants::oauth::INTERVALS_ICU;

    let f = world().await;
    let with_sleep = |tool: &str| {
        let mut params = json!({ "provider": UPLOAD });
        if tool != "generate_recommendations" {
            params["sleep_provider"] = json!(INTERVALS_ICU);
        }
        params
    };
    let nutrition = json!({ "provider": UPLOAD, "recommendation_type": "nutrition" });
    let reads_wellness = [
        ("analyze_training_load", with_sleep("analyze_training_load")),
        (
            "calculate_fitness_score",
            with_sleep("calculate_fitness_score"),
        ),
        ("generate_recommendations", nutrition),
        (
            "estimate_vo2max",
            json!({
                "method": "rockport_walk", "gender": "female", "time_seconds": 780,
                "heart_rate": 132
            }),
        ),
        // A reading against the athlete's saved resting heart rate.
        (
            "calculate_metrics",
            json!({ "provider": UPLOAD, "activity_id": ATHLETE_STREAM_RUN }),
        ),
        // The saved resting heart rate, echoed among the zones' inputs.
        ("calculate_personalized_zones", json!({})),
        // The wellness overrides a document saved before they were refused.
        ("get_user_configuration", json!({})),
    ];
    f.repos()
        .profiles
        .save_configuration(
            &f.athlete.id.to_string(),
            &json!({
                "active_profile": "default",
                "session_overrides": { "resting_hr": 47, "weight_kg": 61.0 }
            })
            .to_string(),
        )
        .await
        .unwrap();

    // Without the grant each is refused its wellness, and records the
    // training read alone.
    for (tool, params) in &reads_wellness {
        let _refused = f
            .try_call(&f.thread_executor(), f.coach, tool, params.clone())
            .await;
        assert_eq!(
            classes(&f, tool).await,
            (1, 0),
            "bug: without the grant {tool} reads no wellness and records none"
        );
    }

    // Under it, each call records its wellness read beside its training read.
    f.share_wellness(true).await;
    for (tool, params) in &reads_wellness {
        ok(f.in_thread(tool, params.clone()).await, tool);
        assert_eq!(
            classes(&f, tool).await,
            (2, 1),
            "bug: under the grant one {tool} call records a training and a wellness row"
        );
    }

    // A call that asks for no wellness records none, grant or not.
    ok(
        f.in_thread("analyze_training_load", json!({ "provider": UPLOAD }))
            .await,
        "analyze_training_load",
    );
    assert_eq!(
        classes(&f, "analyze_training_load").await,
        (3, 1),
        "bug: a training read with no wellness input records the training read alone"
    );
}
