// ABOUTME: The routine engine's gates and waker: mid-conversation deferral, the daily cap, stale and lost wakes
// ABOUTME: Pins ADR-036 — a routine waits for a conversation to end, stops at the cap, and never acts with a write tool

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ADR-036, carnet#1790.
//!
//! The engine decides when every unprompted post fires. These tests drive it
//! through the check-in routine, delivering each queued wake at its own time
//! (`routine_harness`), and pin what the check-in's own suite does not: the
//! gates every routine shares, how a lost or outdated wake behaves, the Cloud
//! Tasks task a wake becomes, and that a routine's turn cannot write.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{CheckInCadence, PostPurpose, RoutineKind, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::services::platform_post::PostAccess;
use pierre_mcp_server::services::routine_engine::{
    HoldReason, WakeOutcome, ATHLETE_DAILY_CAP, MAX_DEFERRALS,
};
use pierre_mcp_server::services::routine_waker::{CloudTasksWaker, SystemJob, Wake, WAKE_PATH};
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

use crate::check_in_post_test::{asked_for, at, athlete, fixture, Fixture};
use crate::helpers::cloud_tasks_stub::{cloud_tasks_runner, NamedQueueStub, QueueStub, TARGET};
use crate::messaging_fixtures::seed_conversation;
use crate::recording_post::RecordingPost;

/// The athlete writes into a conversation of their own at `at`.
async fn wrote(fx: &Fixture, user_id: Uuid, tenant_id: TenantId, at: Timestamp) {
    const SQL: &str = "INSERT INTO chat_messages (id, conversation_id, role, content, created_at) \
                       VALUES ($1, $2, 'user', 'still here', $3)";
    let db = &fx.resources.agent.database;
    let conversation = seed_conversation(db, &user_id.to_string(), tenant_id).await;
    let id = Uuid::new_v4().to_string();
    match db.backend() {
        DatabaseBackend::SQLite(inner) => {
            sqlx::query(SQL)
                .bind(&id)
                .bind(&conversation)
                .bind(at)
                .execute(inner.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(inner) => {
            sqlx::query(SQL)
                .bind(&id)
                .bind(&conversation)
                .bind(at)
                .execute(inner.pool())
                .await
                .unwrap();
        }
    }
}

/// A routine never posts into a conversation the athlete is in: each time
/// they wrote in the last half hour it waits half an hour more, and after
/// three waits the slot is dropped as held. The next day is owed as usual.
#[tokio::test]
async fn a_firing_waits_for_a_conversation_to_end_then_is_held() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    // Writing at 07:50, 08:20, 08:50 and 09:20: always within 30 minutes of
    // the 08:00, 08:30, 09:00 and 09:30 tries.
    for minute in [-10, 20, 50, 80] {
        wrote(
            &fx,
            user_id,
            tenant_id,
            at(0, 8, 0) + Elapsed::minutes(minute),
        )
        .await;
    }

    let delivered = fx.run(at(0, 0, 0), at(0, 10, 0)).await;

    let outcomes: Vec<_> = delivered.iter().map(|(_, outcome)| *outcome).collect();
    assert_eq!(
        outcomes,
        vec![
            WakeOutcome::Deferred(at(0, 8, 30)),
            WakeOutcome::Deferred(at(0, 9, 0)),
            WakeOutcome::Deferred(at(0, 9, 30)),
            WakeOutcome::Held(HoldReason::MidConversation),
        ],
        "{MAX_DEFERRALS} waits, then held"
    );
    assert!(
        asked_for(&fx, user_id).is_empty(),
        "nothing interrupted them"
    );

    fx.run_to(at(1, 9, 0)).await;
    assert_eq!(asked_for(&fx, user_id).len(), 1, "Tuesday's is owed again");
}

/// A conversation that ends inside the waits lets the routine post.
#[tokio::test]
async fn a_firing_posts_once_the_conversation_is_quiet() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    wrote(&fx, user_id, tenant_id, at(0, 7, 50)).await;

    let delivered = fx.run(at(0, 0, 0), at(0, 9, 0)).await;

    assert_eq!(
        delivered[0].1,
        WakeOutcome::Deferred(at(0, 8, 30)),
        "bug: a message at 07:50 did not defer the 08:00 firing: {delivered:?}"
    );
    assert!(
        matches!(delivered[1].1, WakeOutcome::Posted(_)),
        "bug: the 08:30 retry did not post once the conversation went quiet: {delivered:?}"
    );
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "bug: a deferred firing posted more than once"
    );
}

/// An athlete who already had their daily cap of unprompted posts — here two
/// landing reactions before breakfast — gets no check-in that day.
#[tokio::test]
async fn the_daily_cap_holds_a_routine_past_it() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    let claims = &fx.resources.common.repos.platform_post_claims;
    for ride in 0..ATHLETE_DAILY_CAP {
        let subject = format!("strava:{ride}");
        let landed = at(0, 6, 0) + Elapsed::minutes(i64::from(ride));
        let lease = claims
            .claim_platform_post_once(
                tenant_id,
                user_id,
                PostPurpose::ActivityLanded.as_str(),
                &subject,
                landed,
                landed,
            )
            .await
            .unwrap()
            .expect("a fresh claim");
        assert!(
            claims
                .mark_platform_post_posted(
                    tenant_id,
                    user_id,
                    PostPurpose::ActivityLanded.as_str(),
                    &subject,
                    &lease,
                    landed,
                )
                .await
                .unwrap(),
            "bug: a fresh claim could not be marked posted"
        );
    }

    let delivered = fx.run(at(0, 0, 0), at(0, 9, 0)).await;

    assert_eq!(
        delivered[0].1,
        WakeOutcome::Held(HoldReason::DailyCap),
        "bug: a firing past the daily cap was not held: {delivered:?}"
    );
    assert!(
        asked_for(&fx, user_id).is_empty(),
        "bug: a firing past the daily cap posted"
    );
    fx.run_to(at(1, 9, 0)).await;
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "the cap is per local day: Tuesday's goes out"
    );
}

/// A wake Cloud Tasks refused is queued again by the next reconcile — the
/// 3-hourly cron's — and fires on time.
#[tokio::test]
async fn a_lost_wake_is_queued_again_by_the_reconcile() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, _) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    fx.wakes.refuse_next(1);

    fx.engine.reconcile(at(0, 0, 0)).await.unwrap();
    let routine_wakes = |fx: &Fixture| {
        fx.wakes
            .pending()
            .into_iter()
            .filter(|wake| matches!(wake, Wake::Routine { .. }))
            .count()
    };
    assert_eq!(routine_wakes(&fx), 0, "the wake was lost");

    let delivered = fx.run(at(0, 3, 0), at(0, 9, 0)).await;
    assert!(
        matches!(delivered[0].1, WakeOutcome::Posted(_)),
        "{delivered:?}"
    );
    assert_eq!(
        delivered[0].0.fire_at(),
        Some(at(0, 8, 0)),
        "bug: the re-queued wake is not for the routine's 08:00 slot"
    );
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "bug: the re-queued wake did not post exactly once"
    );
}

/// A wake queued for a schedule the athlete has since changed finds nothing
/// to fire: setting the cadence unschedules the routine, and the engine
/// schedules the new cadence's slot.
#[tokio::test]
async fn a_wake_for_a_changed_schedule_is_stale() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    fx.run(at(1, 0, 0), at(1, 0, 0)).await;
    let tuesday = fx
        .wakes
        .pending()
        .into_iter()
        .find(|wake| wake.fire_at() == Some(at(1, 8, 0)))
        .expect("Tuesday's 08:00 is queued");

    fx.resources
        .common
        .repos
        .check_in_preferences
        .set_check_in_cadence(tenant_id, user_id, CheckInCadence::Weekly, at(1, 1, 0))
        .await
        .unwrap();
    fx.engine
        .reschedule(tenant_id, user_id, RoutineKind::CheckIn, at(1, 1, 0))
        .await;

    assert_eq!(
        fx.engine.handle(tuesday, at(1, 8, 0)).await.unwrap(),
        WakeOutcome::Stale,
        "bug: a wake for the old cadence still fired"
    );
    assert!(
        fx.wakes
            .pending()
            .iter()
            .any(|wake| wake.fire_at() == Some(at(7, 8, 0))),
        "next Monday is queued"
    );
    fx.run_to(at(7, 9, 0)).await;
    assert_eq!(
        asked_for(&fx, user_id).len(),
        1,
        "only Monday's weekly check-in"
    );
}

/// Turning the check-in off pauses the routine: its queued wake is stale.
#[tokio::test]
async fn turning_the_check_in_off_stops_the_queued_wake() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    fx.engine.reconcile(at(0, 0, 0)).await.unwrap();

    fx.resources
        .common
        .repos
        .check_in_preferences
        .set_check_in_cadence(tenant_id, user_id, CheckInCadence::Off, at(0, 1, 0))
        .await
        .unwrap();

    let delivered = fx.run_to(at(1, 9, 0)).await;
    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert_eq!(
        delivered[0].1,
        WakeOutcome::Stale,
        "bug: the wake queued before `off` still fired"
    );
    assert!(
        asked_for(&fx, user_id).is_empty(),
        "bug: a check-in turned off posted"
    );
}

/// A wake becomes one Cloud Tasks HTTP task: named by what it fires and
/// when, scheduled for that time, posted to the wake route with the service's
/// own OIDC identity, carrying the wake as its body.
#[tokio::test]
async fn a_wake_is_a_named_task_scheduled_for_its_time() {
    use pierre_mcp_server::services::routine_waker::RoutineWaker;

    let stub = QueueStub::accepting();
    let api = stub.serve().await;
    let runner = cloud_tasks_runner(&api, "http://127.0.0.1:9/jwks", Duration::from_secs(5));
    let waker = CloudTasksWaker::over(&runner);
    let routine = Uuid::new_v4();
    let wake = Wake::routine(Uuid::new_v4(), routine, at(0, 8, 0));

    waker.queue(wake).await.unwrap();

    let received = stub.received();
    assert_eq!(
        received.len(),
        1,
        "bug: one wake did not become exactly one task"
    );
    let task = &received[0];
    assert!(
        task.name().ends_with(&format!(
            "/tasks/routine-{routine}-{}",
            at(0, 8, 0).epoch_seconds()
        )),
        "bug: the task is not named by what it fires and when: {}",
        task.name()
    );
    assert_eq!(
        task.url(),
        format!("{TARGET}{WAKE_PATH}"),
        "bug: the task is not delivered to the wake route"
    );
    assert_eq!(
        Timestamp::parse_from_rfc3339(task.body["task"]["scheduleTime"].as_str().unwrap()).unwrap(),
        at(0, 8, 0),
        "bug: the task is not scheduled for the wake's time"
    );
    assert_eq!(
        task.body["task"]["httpRequest"]["oidcToken"]["audience"],
        json!(TARGET),
        "bug: the task does not carry the service's own OIDC audience"
    );
    let delivered: Wake = serde_json::from_value(task.delivery_body()).unwrap();
    assert_eq!(
        delivered, wake,
        "bug: the task's body is not the wake it was queued for"
    );
}

/// Queuing a wake that is already queued is not an error and queues nothing
/// new: the reconcile re-queues every pending wake without duplicating any.
#[tokio::test]
async fn queuing_a_queued_wake_again_is_a_no_op() {
    use pierre_mcp_server::services::routine_waker::RoutineWaker;

    let stub = NamedQueueStub::new();
    let api = stub.serve().await;
    let runner = cloud_tasks_runner(&api, "http://127.0.0.1:9/jwks", Duration::from_secs(5));
    let waker = CloudTasksWaker::over(&runner);
    let wake = Wake::system(SystemJob::PersonaDigest, at(0, 11, 0));

    waker.queue(wake).await.unwrap();
    waker.queue(wake).await.unwrap();

    assert_eq!(
        stub.created().len(),
        1,
        "bug: queuing a queued wake again created a second task"
    );
}

/// A routine's turn runs on a read-only grant, so the tool layer refuses a
/// write tool whatever the model asks for.
#[tokio::test]
async fn a_routine_turn_cannot_run_a_write_tool() {
    let fx = fixture(RecordingPost::posting(true)).await;
    let (user_id, tenant_id) = athlete(&fx, Some(CheckInCadence::Daily), None).await;
    let runtime: Arc<dyn ToolRuntime> = fx.resources.clone();
    let request = || UniversalRequest {
        tool_name: "set_goal".to_owned(),
        parameters: json!({ "goal_type": "distance", "target_value": 100.0, "timeframe": "month" }),
        user_id: user_id.to_string(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
    };

    let routine =
        UniversalExecutor::new(Arc::clone(&runtime)).with_scopes(PostAccess::SuggestOnly.grant());
    let refused = routine
        .execute_tool(request())
        .await
        .expect_err("a read-only grant runs no write tool");
    assert!(
        refused.to_string().contains("fitness:write"),
        "refused for the scope, before the tool reads anything: {refused}"
    );

    // A post the athlete asked for keeps their own grant: the same write
    // tool is not refused for its scope.
    let owner = UniversalExecutor::new(Arc::clone(&runtime)).with_scopes(PostAccess::Owner.grant());
    if let Err(e) = owner.execute_tool(request()).await {
        assert!(
            !e.to_string().contains("fitness:write"),
            "bug: a post the athlete asked for was refused the write scope: {e}"
        );
    }
    assert!(
        PostAccess::SuggestOnly
            .grant()
            .iter()
            .all(|scope| !matches!(
                scope,
                OAuthScope::FitnessWrite | OAuthScope::ProfileWrite | OAuthScope::Admin
            )),
        "bug: a routine's grant carries a write or admin scope"
    );
}
