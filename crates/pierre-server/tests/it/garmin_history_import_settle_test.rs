// ABOUTME: How a one-time Garmin history import settles: what it adopts, retries, keeps, pages and never announces
// ABOUTME: Pins the import as a recorded property of its job, the wellness-only session, the card states and the digest guard

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Settling a one-time history import (carnet#1386, phase 2).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// An import is what its own ask recorded, never what the rows standing
// beside a job make it look like: an ordinary backfill owed before the grant
// stays ordinary when resumed, and one the import finds in its way is adopted
// as the import instead of ending it at its own narrow window. A transient
// failure keeps the session for the next run. A grant that withholds the
// health export never retires the session, which is then the athlete's only
// wellness source. The card reads `importing` while the job is owed. And an
// import copies the past: its write-through announces no session.
#![cfg(all(feature = "provider-garmin", feature = "health-sync"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_history_import_test::{
    athlete, calls_matching, connections, garmin_history, hold_grant, hold_session, import_ended,
    job_row, job_state, rows_under, server, session_token_held, sign_in, spawn_switchable_scraper,
    Calls, ScraperSwitch, IMPORT_DEADLINE, LIVE_SESSION, SIGNED_IN_SESSION,
};

use std::slice;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{
    ActivityBuilder, CoachingPersona, ConnectionStatus, ConnectionType, SportType, TenantId,
    PERMISSION_WITHDRAWN,
};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, BackfillJobState, JobLease,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::models::TransportPolicy as PushTransportPolicy;
use pierre_notifications::{
    DispatchOutcome, DispatchRequest, NotificationEvent, PushTier, TenantId as CommTenantId,
};
use pierre_providers::core::ActivityQueryParams;
use pierre_services::history_import;
use pierre_services::provider_revocation::{retire_history_session, HistoryRetirement};
use pierre_tool_runtime::activity_backfill_resume::resume_backfill_jobs;
use pierre_tool_runtime::activity_fetch::write_through::write_through_served_window;
use pierre_tool_runtime::history_import::{start_history_import, WHOLE_ACCOUNT_AFTER};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;

/// An ordinary backfill of the session, as a chat or the capture rail asks
/// for one: a one-year window, a conversation to answer.
fn ordinary_row(
    user_id: Uuid,
    tenant: TenantId,
    created_at_ms: i64,
    leased_until_ms: i64,
) -> ActivityBackfillJobRow {
    ActivityBackfillJobRow {
        after_ts: Some(Utc::now().timestamp() - 365 * 86_400),
        fetch_limit: Some(200),
        conversation_id: Some("conversation-that-asked".to_owned()),
        created_at_ms,
        leased_until_ms,
        lease_token: Some(Uuid::new_v4().to_string()),
        history_import: false,
        ..job_row(user_id, tenant, created_at_ms)
    }
}

/// Claim every lapsed row as the sweep would, an hour from now.
async fn claim_all(resources: &Arc<ServerContext>) -> Vec<ActivityBackfillJobRow> {
    let now_ms = Utc::now().timestamp_millis() + 3_600_000;
    resources
        .common
        .repos
        .activity_backfill_jobs
        .claim_stale_backfill_jobs(BackfillJobClaim {
            now_ms,
            queued_older_than_ms: 0,
            lease_ms: 60_000,
            max_attempts: 10,
            limit: 10,
        })
        .await
        .unwrap()
}

/// A sign-in's import finds an ordinary backfill already owed for the
/// session — one an earlier chat asked for, still leased to the run on
/// another instance. The import adopts it: whole account, no `before`, no
/// conversation to answer, attempts reset, and recorded as the import, so
/// the run holding it can no longer finish it and the sweep runs the import.
#[tokio::test]
async fn an_import_adopts_the_ordinary_job_owed_for_its_session() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let now = Utc::now().timestamp_millis();
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&ordinary_row(user_id, tenant, now - 600_000, now + 600_000))
            .await
            .unwrap(),
        "bug: the ledger records the ordinary job"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert!(
        start_history_import(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "bug: an import is on file for the session"
    );
    assert!(
        !start_history_import(&runtime, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        "a second ask finds the import already on file"
    );

    let claimed = claim_all(&resources).await;
    assert_eq!(claimed.len(), 1, "bug: exactly the owed row is claimed");
    let row = &claimed[0];
    assert!(row.history_import, "bug: the owed job is now the import");
    assert_eq!(
        row.after_ts,
        Some(WHOLE_ACCOUNT_AFTER),
        "bug: the import asks for the whole account"
    );
    assert_eq!(
        row.before_ts, None,
        "bug: the import's first page has no upper bound"
    );
    assert_eq!(row.conversation_id, None, "bug: an import answers no chat");
    assert_eq!(
        row.attempts, 1,
        "the adopted import starts its attempts over"
    );
}

/// An ordinary backfill owed for the session since before the grant is
/// resumed as what it was: it is not taken for an import because a grant now
/// stands beside it, so its end retires nothing.
#[tokio::test]
async fn a_resumed_ordinary_backfill_beside_a_grant_is_no_import() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&ordinary_row(user_id, tenant, long_ago, 0))
            .await
            .unwrap(),
        "bug: the ledger records the ordinary job"
    );

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    assert_eq!(
        resume_backfill_jobs(runtime).await.unwrap(),
        1,
        "bug: the sweep resumes the owed import"
    );
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while job_state(&resources, user_id, tenant).await.is_some() {
        assert!(
            Instant::now() < deadline,
            "the resumed backfill never ended"
        );
        sleep(Duration::from_millis(50)).await;
    }

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: an ordinary backfill never retires the session"
    );
    assert!(
        calls_matching(&calls, &format!("activities {LIVE_SESSION}"))
            .iter()
            .all(|call| !call.contains(&WHOLE_ACCOUNT_AFTER.to_string())),
        "bug: the ordinary backfill keeps its own window"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// The scraper fails the import's read once: the job stays owed for the
/// resume sweep and the session stays for it — nothing is retired or marked
/// incomplete on a hiccup — and the card reads `importing` meanwhile.
#[tokio::test]
async fn a_transient_failure_keeps_the_session_and_the_card_reads_importing() {
    let switch = ScraperSwitch::default();
    switch.fail_next.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while calls_matching(&calls, &format!("activities {SIGNED_IN_SESSION}")).is_empty() {
        assert!(Instant::now() < deadline, "the import never read");
        sleep(Duration::from_millis(50)).await;
    }
    sleep(Duration::from_millis(500)).await;

    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: a transient failure leaves the import owed"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a transient failure never retires the session"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "importing",
        "bug: the card reads importing while the job is owed"
    );
}

/// The card's order: rows under the session's key read `imported` even when
/// the last import ended incomplete — the athlete keeps what was copied.
#[tokio::test]
async fn rows_under_the_session_key_read_imported_over_an_incomplete_end() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let mut row = job_row(user_id, tenant, 0);
    row.lease_token = Some(Uuid::new_v4().to_string());
    assert!(
        ledger.record_backfill_job(&row).await.unwrap(),
        "bug: the ledger records the job"
    );
    let lease = JobLease {
        id: row.id.clone(),
        token: row.lease_token.clone().unwrap(),
    };
    assert!(
        ledger.end_backfill_job_incomplete(&lease).await.unwrap(),
        "bug: the lease holder ends its job incomplete"
    );
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "incomplete",
        "bug: an import ended before its history landed reads incomplete"
    );

    let ride = ActivityBuilder::new(
        "7000000009",
        "Garmin ride",
        SportType::Ride,
        Utc::now() - ChronoDuration::days(400),
        3_600,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &[ride],
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "imported",
        "bug: what the session copied reads imported, whatever the last import did"
    );
}

/// What the grant pushed itself starts the day it connected, so it never
/// stands for the history before it: the card still offers the import.
#[tokio::test]
async fn the_grants_own_pushes_leave_the_history_needed() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let pushed = ActivityBuilder::new(
        "7000000010",
        "Pushed ride",
        SportType::Ride,
        Utc::now() - chrono::Duration::hours(3),
        3_600,
        oauth_providers::GARMIN,
    )
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            user_id,
            &tenant,
            oauth_providers::GARMIN,
            &[pushed],
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        garmin_history(&resources, &token).await["state"],
        "needed",
        "bug: a grant's own pushes read as the history before it"
    );
}

/// A grant that withholds the health export leaves the session the athlete's
/// only wellness source: the import copies the activities, and the session
/// stays.
#[tokio::test]
async fn a_grant_without_the_health_export_keeps_the_session() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    resources
        .common
        .repos
        .provider_connections
        .set_provider_grants(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            &["ACTIVITY_EXPORT".to_owned()],
        )
        .await
        .unwrap();

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while job_state(&resources, user_id, tenant).await.is_some()
        || rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await == 0
    {
        assert!(Instant::now() < deadline, "the import never ended");
        sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "the activities are copied all the same"
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the session that alone brings the wellness in is kept"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
    assert_eq!(
        retire_history_session(
            &resources.common.repos,
            None,
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
        )
        .await
        .unwrap(),
        HistoryRetirement::KeptForWellness,
        "bug: no caller retires it either"
    );
}

/// A refused grant is the stronger state: a restored permission never
/// re-arms it, since only a flag raised for the withdrawal is the
/// withdrawal's to lift, while a refresh that proves the token alive does.
#[tokio::test]
async fn a_refused_grant_is_lifted_by_a_refresh_never_by_a_restored_permission() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    let connections_repo = &resources.common.repos.provider_connections;
    connections_repo
        .mark_needs_reauth(
            user_id,
            tenant,
            oauth_providers::GARMIN,
            Some("grant_refused"),
            Utc::now(),
        )
        .await
        .unwrap();

    assert!(
        !connections_repo
            .mark_active_if_flagged_for(
                user_id,
                tenant,
                oauth_providers::GARMIN,
                PERMISSION_WITHDRAWN,
            )
            .await
            .unwrap(),
        "bug: a restored permission lifts a refused grant's flag"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(
            oauth_providers::GARMIN.to_owned(),
            ConnectionStatus::NeedsReauth
        )],
        "bug: the refused grant stays flagged"
    );
    assert!(
        connections_repo
            .mark_active_unless_flagged_for(
                user_id,
                tenant,
                oauth_providers::GARMIN,
                PERMISSION_WITHDRAWN,
            )
            .await
            .unwrap(),
        "bug: a refresh that proves the token alive re-arms a refused grant"
    );
}

/// The finder never lists a delegated Garmin connection: a coach's view of
/// a member is no session of the athlete's to retire.
#[tokio::test]
async fn the_sessions_beside_grant_finder_skips_delegated_rows() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &ConnectionType::Delegated,
            None,
        )
        .await
        .unwrap();
    let listed = resources
        .common
        .repos
        .provider_connections
        .list_sessions_beside_grant(
            tenant,
            oauth_providers::SCIOTTE_GARMIN,
            oauth_providers::GARMIN,
        )
        .await
        .unwrap();
    assert_eq!(
        listed,
        Vec::<Uuid>::new(),
        "bug: a delegated row is no session"
    );
}

/// A history import copies the past: its write-through announces no session,
/// so an athlete with an armed per-session digest and pushes held for it gets
/// no digest from the copy — while an ordinary write of a new session still
/// sends one.
#[tokio::test]
async fn an_import_never_sends_the_per_session_digest() {
    let (scraper, _calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = common::create_test_server_resources_with_config(
        common::test_server_config_with_sciotte(&scraper),
    )
    .await
    .unwrap();
    resources
        .fitness
        .persona_contract_registry
        .apply_overlay(
            "version: 1\npersonas:\n  power_athlete:\n    notification:\n      tier_floor: P2\n      digest: per_session\n",
        )
        .unwrap();
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    let repos = &resources.common.repos;
    repos
        .users
        .set_coaching_persona(user_id, CoachingPersona::PowerAthlete)
        .await
        .unwrap();
    repos
        .feature_flags
        .set_user_override(user_id, FeatureKey::PersonaNotificationPolicy, true, None)
        .await
        .unwrap();
    let service = resources
        .common
        .notification_service
        .clone()
        .expect("the server wires a notification service");
    let held = service
        .dispatch_with_tier(
            &DispatchRequest {
                user_id,
                tenant_id: CommTenantId(tenant.as_uuid()),
                category: NotificationCategory::Training,
                notification_type: "training_load_alert".to_owned(),
                title: "Training load elevated".to_owned(),
                body: "held for the digest".to_owned(),
                data: Some(json!({ "screen": "coach" })),
                image_url: None,
                actions: None,
                bypass_frequency_cap: false,
                transport_policy: PushTransportPolicy::AnyTransport,
            },
            PushTier::P3,
        )
        .await
        .unwrap();
    assert!(
        matches!(held, DispatchOutcome::PersistedNoDevices { .. }),
        "the armed gate holds the push: {held:?}"
    );
    let digests = || {
        let service = service.clone();
        async move {
            let (rows, _, _) = service
                .list_notifications(user_id, CommTenantId(tenant.as_uuid()), 50, 0, None, false)
                .await
                .unwrap();
            rows.iter()
                .filter(|n| n.notification_type == NotificationEvent::PersonaSessionDigest.wire())
                .count()
        }
    };

    hold_grant(&resources, user_id, tenant).await;
    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    import_ended(&resources, user_id, tenant).await;
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "the copy landed, its ride three days old the newest in the cache"
    );
    sleep(Duration::from_millis(500)).await;
    assert_eq!(digests().await, 0, "bug: a history import sends no digest");

    // Control: a new session written by an ordinary read does send it.
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ride = ActivityBuilder::new(
        "new-ride",
        "Morning ride",
        SportType::Ride,
        Utc::now() - chrono::Duration::minutes(10),
        3_600,
        oauth_providers::GARMIN,
    )
    .build();
    write_through_served_window(
        &runtime,
        user_id,
        &tenant,
        oauth_providers::GARMIN,
        &ActivityQueryParams::default(),
        slice::from_ref(&ride),
    )
    .await;
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while digests().await == 0 {
        assert!(
            Instant::now() < deadline,
            "the control session never sent its digest"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// The operator one-off behind `pierre-cli garmin retire-history-sessions`:
/// a dry run lists the athletes and touches nothing; applying it retires the
/// session of a grant that pushes wellness, keeps the one of a grant that
/// withholds it, and leaves every cached activity where it was.
#[tokio::test]
async fn retire_history_sessions_dry_runs_then_retires_without_a_purge() {
    let resources = server(common::test_server_config()).await;
    let repos = &resources.common.repos;
    let (retired, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, retired, tenant).await;
    hold_session(&resources, retired, tenant, LIVE_SESSION).await;
    let ride = ActivityBuilder::new(
        "7000000011",
        "Garmin ride",
        SportType::Ride,
        Utc::now() - ChronoDuration::days(900),
        3_600,
        oauth_providers::SCIOTTE_GARMIN,
    )
    .build();
    repos
        .activity_cache
        .upsert_activities_synced_at(
            retired,
            &tenant,
            oauth_providers::SCIOTTE_GARMIN,
            &[ride],
            Utc::now(),
        )
        .await
        .unwrap();
    // A second athlete of the tenant whose grant withholds the health export.
    let (kept, _user, _own_tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, kept, tenant).await;
    hold_session(&resources, kept, tenant, LIVE_SESSION).await;
    repos
        .provider_connections
        .set_provider_grants(
            kept,
            tenant,
            oauth_providers::GARMIN,
            &["ACTIVITY_EXPORT".to_owned()],
        )
        .await
        .unwrap();

    let dry = history_import::retire_history_sessions(repos, None, tenant, true)
        .await
        .unwrap();
    assert_eq!(dry.len(), 2, "{dry:?}");
    assert!(
        dry.iter().all(|report| report.retirement.is_none()),
        "bug: a dry run retires nothing"
    );
    assert!(
        dry.iter().any(|r| r.user_id == kept && r.kept_for_wellness),
        "bug: a dry run predicts the wellness source kept"
    );
    assert!(
        session_token_held(&resources, retired, tenant).await,
        "bug: a dry run retires nothing"
    );

    let applied = history_import::retire_history_sessions(repos, None, tenant, false)
        .await
        .unwrap();
    let report = applied.iter().find(|r| r.user_id == retired).unwrap();
    assert_eq!(
        report.retirement,
        Some(HistoryRetirement::Retired),
        "bug: the session beside a wellness-sharing grant is retired"
    );
    assert_eq!(
        report.after.as_ref(),
        Some(&report.before),
        "bug: retirement purges no cached activity"
    );
    assert_eq!(
        rows_under(&resources, retired, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        1,
        "bug: retirement keeps every row the session copied"
    );
    assert!(
        !session_token_held(&resources, retired, tenant).await,
        "bug: a retired session keeps no token row"
    );
    let kept_report = applied.iter().find(|r| r.user_id == kept).unwrap();
    assert_eq!(
        kept_report.retirement,
        Some(HistoryRetirement::KeptForWellness),
        "bug: the only wellness source is kept"
    );
    assert!(
        session_token_held(&resources, kept, tenant).await,
        "bug: the only wellness source keeps its token row"
    );
}

/// The whole account in pages: an import whose read fills its fetch limit
/// leaves its row owed for the page below the oldest activity it reached,
/// with the session kept for it; the next page is read under that `before`,
/// and only the page that reaches nothing older ends the import and retires
/// the session.
#[tokio::test]
async fn an_import_that_fills_its_limit_pages_down_before_it_retires() {
    let (scraper, calls) = spawn_switchable_scraper(ScraperSwitch::default()).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;
    hold_session(&resources, user_id, tenant, LIVE_SESSION).await;
    let long_ago = Utc::now().timestamp_millis() - 600_000;
    let mut row = job_row(user_id, tenant, long_ago);
    // The stand-in's account holds three activities: a limit of three fills.
    row.fetch_limit = Some(3);
    assert!(
        resources
            .common
            .repos
            .activity_backfill_jobs
            .record_backfill_job(&row)
            .await
            .unwrap(),
        "bug: the ledger records the import"
    );
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let reads = || calls_matching(&calls, &format!("activities {LIVE_SESSION}"));

    assert_eq!(
        resume_backfill_jobs(Arc::clone(&runtime)).await.unwrap(),
        1,
        "bug: the sweep resumes the owed import"
    );
    let deadline = Instant::now() + IMPORT_DEADLINE;
    loop {
        let paged = resources
            .common
            .repos
            .activity_backfill_jobs
            .claim_stale_backfill_jobs(BackfillJobClaim {
                now_ms: Utc::now().timestamp_millis(),
                queued_older_than_ms: 0,
                lease_ms: 0,
                max_attempts: 10,
                limit: 10,
            })
            .await
            .unwrap();
        if let Some(paged) = paged.first() {
            assert_eq!(
                paged.before_ts,
                Some(OLDEST_RUN_TS),
                "bug: the next page reads below the oldest activity reached"
            );
            assert!(
                paged.history_import,
                "bug: a paged-down row stays a history import"
            );
            break;
        }
        assert!(Instant::now() < deadline, "the first page never paged down");
        sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(reads().len(), 1, "bug: one page is one session read");
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: the session stays for the next page"
    );
    assert_eq!(
        rows_under(&resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await,
        3,
        "bug: every page's rows land under the session's key"
    );

    // The claim above leased the row for no time at all: the sweep takes it
    // again and reads the next page, which reaches nothing older.
    assert_eq!(
        resume_backfill_jobs(runtime).await.unwrap(),
        1,
        "bug: the sweep resumes the owed import"
    );
    import_ended(&resources, user_id, tenant).await;
    let reads = reads();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert!(
        reads[1].contains(&format!("before={OLDEST_RUN_TS}")),
        "bug: the second page is read below the first: {reads:?}"
    );
    assert_eq!(
        connections(&resources, user_id, tenant).await,
        vec![(oauth_providers::GARMIN.to_owned(), ConnectionStatus::Active)],
        "bug: only the grant's connection is left"
    );
}

/// 2012-05-06T08:00:00Z, the stand-in account's oldest run.
const OLDEST_RUN_TS: i64 = 1_336_291_200;

/// The activities land but every wellness read fails: the session that holds
/// those nights is not retired and the import stays owed for the next run —
/// a hiccup never costs the athlete their sleep history.
#[tokio::test]
async fn a_failed_wellness_copy_keeps_the_session_for_the_next_run() {
    let switch = ScraperSwitch::default();
    switch.fail_summaries.store(true, Ordering::SeqCst);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    wellness_read_settled(&resources, &calls, user_id, tenant).await;

    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a failed wellness copy never retires the session"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the import stays owed for the next run"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// One day of the year before the grant fails to read while every other
/// lands: the copy is not complete, so the session that still holds that
/// day is kept and the import stays owed, rather than retired over a hole.
#[tokio::test]
async fn one_unread_wellness_day_keeps_the_session_for_the_next_run() {
    let switch = ScraperSwitch::default();
    let hole = Utc::now().date_naive() - ChronoDuration::days(200);
    *switch.fail_summary_on.lock().unwrap() = Some(hole);
    let (scraper, calls) = spawn_switchable_scraper(switch).await;
    let resources = server(common::test_server_config_with_sciotte(&scraper)).await;
    let (user_id, _user, tenant, token) = athlete(&resources).await;
    hold_grant(&resources, user_id, tenant).await;

    let (status, body) = sign_in(&resources, &token).await;
    assert_eq!(status, 200, "{body}");
    wellness_read_settled(&resources, &calls, user_id, tenant).await;

    let landed = resources
        .common
        .repos
        .recovery
        .get_recovery_metrics(
            user_id,
            &tenant,
            Utc::now() - ChronoDuration::days(400),
            Utc::now() + ChronoDuration::days(1),
        )
        .await
        .unwrap();
    assert!(
        landed.len() > 300 && landed.iter().all(|morning| morning.date != hole),
        "the other days landed, the failed one did not: {} mornings",
        landed.len()
    );
    assert!(
        session_token_held(&resources, user_id, tenant).await,
        "bug: a copy with an unread day never retires the session"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the import stays owed for the next run"
    );
    assert_eq!(
        calls_matching(&calls, "drop"),
        Vec::<String>::new(),
        "bug: nothing drops the session"
    );
}

/// Wait until the import's activities landed, its wellness read reached
/// every day of the year before the grant, and its job row is back to owed.
async fn wellness_read_settled(
    resources: &Arc<ServerContext>,
    calls: &Calls,
    user_id: Uuid,
    tenant: TenantId,
) {
    let deadline = Instant::now() + IMPORT_DEADLINE;
    while calls_matching(calls, &format!("summary {SIGNED_IN_SESSION}")).len() < 365
        || rows_under(resources, user_id, tenant, oauth_providers::SCIOTTE_GARMIN).await < 3
        || job_state(resources, user_id, tenant).await != Some(BackfillJobState::Owed)
    {
        assert!(
            Instant::now() < deadline,
            "the import never settled after its wellness"
        );
        sleep(Duration::from_millis(50)).await;
    }
}

/// A claim the sweep cannot run is handed back with its attempt, so waiting
/// on a run already in flight never spends the job's attempts.
#[tokio::test]
async fn a_released_claim_returns_its_attempt() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, 0))
            .await
            .unwrap(),
        "bug: the ledger records the job"
    );
    let claimed = claim_all(&resources).await;
    assert_eq!(
        claimed[0].attempts, 2,
        "bug: the claim counts one more attempt"
    );
    let lease = JobLease {
        id: claimed[0].id.clone(),
        token: claimed[0].lease_token.clone().unwrap(),
    };
    assert!(
        ledger.release_backfill_job_claim(&lease).await.unwrap(),
        "bug: the lease holder hands its claim back"
    );
    assert!(
        !ledger.release_backfill_job_claim(&lease).await.unwrap(),
        "a released claim holds the row no more"
    );
    let again = claim_all(&resources).await;
    assert_eq!(again.len(), 1, "bug: a released row is claimable at once");
    assert_eq!(
        again[0].attempts, 2,
        "bug: the release returned the attempt"
    );
}

/// A row that records how one tenant's import ended is never replaced by an
/// ask for the same user and provider from another tenant: the first
/// tenant's card keeps reading what its own import did.
#[tokio::test]
async fn an_ask_from_another_tenant_never_replaces_an_ended_row() {
    let resources = server(common::test_server_config()).await;
    let (user_id, _user, tenant, _token) = athlete(&resources).await;
    let (_other_user, _user, other_tenant, _token) = athlete(&resources).await;
    let ledger = &resources.common.repos.activity_backfill_jobs;
    let mut row = job_row(user_id, tenant, 0);
    row.lease_token = Some(Uuid::new_v4().to_string());
    assert!(
        ledger.record_backfill_job(&row).await.unwrap(),
        "bug: the ledger records the job"
    );
    let lease = JobLease {
        id: row.id.clone(),
        token: row.lease_token.clone().unwrap(),
    };
    assert!(
        ledger.end_backfill_job_incomplete(&lease).await.unwrap(),
        "bug: the lease holder ends its job incomplete"
    );

    assert!(
        !ledger
            .record_backfill_job(&job_row(user_id, other_tenant, 0))
            .await
            .unwrap(),
        "bug: an ask from another tenant replaced this tenant's ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Incomplete),
        "bug: this tenant's card no longer reads its own import's end"
    );
    assert_eq!(
        job_state(&resources, user_id, other_tenant).await,
        None,
        "bug: the other tenant took the row over"
    );

    assert!(
        ledger
            .record_backfill_job(&job_row(user_id, tenant, 0))
            .await
            .unwrap(),
        "bug: an ask from the row's own tenant replaces its ended row"
    );
    assert_eq!(
        job_state(&resources, user_id, tenant).await,
        Some(BackfillJobState::Owed),
        "bug: the own tenant's new ask is owed"
    );
}
