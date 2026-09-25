// ABOUTME: Covers the usage, usage-counter and LLM-usage repositories against whichever backend DATABASE_URL names
// ABOUTME: Pins the request-log round trip, the JWT month window, the counter timestamp, the llm_usage read-back and the since contract
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `api_key_usage`, `jwt_usage`, `usage_counters` and `llm_usage` are each
//! written once and emitted for both backends, differing only in how a uuid
//! reaches a column, the cast an IP address needs, how a `SERIAL` id is
//! left to the table and how the UTC day of a row is spelled.
//!
//! Every assertion here pins something the two halves used to disagree on
//! or that no test read back: the request log's stored id, sizes and
//! saturated response time; the case-insensitive tool filter; the JWT
//! month-to-date window against a row from last month; the counter's
//! `updated_at` shape; an `llm_usage` row reading back exactly as its
//! insert returned it; a key's window count and its oldest call; and a
//! `since` given as a bare day or as garbage.
//! These run on `SQLite` and on `PostgreSQL`: `create_test_db` opens
//! whichever `DATABASE_URL` names, so the same assertions cover both.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{DateTime, Datelike, Duration, SubsecRound, Utc};
use pierre_core::errors::ErrorCode;
use pierre_core::models::usage::InsertLlmUsage;
use pierre_core::models::{
    ApiKey, ApiKeyTier, ApiKeyUsage, ApiKeyWindowUsage, ConversationTurnId, JwtUsage, TenantId,
    User,
};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::repositories::analytics::{next_utc_day_start, utc_day_start};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// A distinct user per call, so one test's rows cannot satisfy another's
/// assertions; `api_keys` and `jwt_usage` both reference `users`.
async fn fresh_user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("usage-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Usage Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

/// An active key for `user_id` with a one-hour rate-limit window.
async fn fresh_api_key(repos: &RepositoryRegistry, user_id: Uuid) -> ApiKey {
    let api_key = ApiKey {
        id: Uuid::new_v4().to_string(),
        user_id,
        name: "usage-key".to_owned(),
        key_prefix: format!("pk_{}", &Uuid::new_v4().simple().to_string()[..8]),
        key_hash: Uuid::new_v4().to_string(),
        description: None,
        tier: ApiKeyTier::Starter,
        rate_limit_requests: 1000,
        rate_limit_window_seconds: 3600,
        is_active: true,
        last_used_at: None,
        expires_at: None,
        created_at: Utc::now(),
    };
    repos.api_keys.create(&api_key).await.unwrap();
    api_key
}

fn api_call(api_key_id: &str, tool_name: &str, status_code: u16, at: DateTime<Utc>) -> ApiKeyUsage {
    ApiKeyUsage {
        id: None,
        api_key_id: api_key_id.to_owned(),
        timestamp: at,
        tool_name: tool_name.to_owned(),
        response_time_ms: Some(50),
        status_code,
        error_message: None,
        request_size_bytes: Some(256),
        response_size_bytes: Some(1024),
        ip_address: Some("203.0.113.7".to_owned()),
        user_agent: Some("usage-test/1.0".to_owned()),
    }
}

fn jwt_call(user_id: Uuid, at: DateTime<Utc>) -> JwtUsage {
    JwtUsage {
        id: None,
        user_id,
        timestamp: at,
        endpoint: "/api/activities".to_owned(),
        method: "GET".to_owned(),
        status_code: 200,
        response_time_ms: Some(12),
        request_size_bytes: Some(0),
        response_size_bytes: Some(2048),
        ip_address: Some("203.0.113.7".to_owned()),
        user_agent: Some("usage-test/1.0".to_owned()),
    }
}

#[tokio::test]
async fn an_api_call_reads_back_from_the_request_log_with_what_was_stored() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let api_key = fresh_api_key(&repos, user_id).await;
    let repo = &repos.usage;

    let failed = ApiKeyUsage {
        error_message: Some("upstream timed out".to_owned()),
        response_time_ms: Some(u32::MAX),
        ..api_call(&api_key.id, "get_athlete", 504, Utc::now())
    };
    repo.record_api_key(&api_call(
        &api_key.id,
        "get_activities",
        200,
        Utc::now() - Duration::minutes(5),
    ))
    .await
    .unwrap();
    repo.record_api_key(&failed).await.unwrap();

    let logs = repo
        .get_request_logs(Some(user_id), None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(logs.len(), 2, "both calls are the user's");
    assert_eq!(logs[0].tool_name, "get_athlete", "newest first");
    assert_eq!(logs[0].api_key_id, api_key.id);
    assert_eq!(
        logs[0].api_key_name, "usage-key",
        "the key's name rides on the join"
    );
    assert_eq!(logs[0].status_code, 504);
    assert_eq!(
        logs[0].response_time_ms,
        Some(i32::MAX),
        "a response time past the column saturates instead of refusing the row"
    );
    assert_eq!(logs[0].error_message.as_deref(), Some("upstream timed out"));
    assert_eq!(logs[0].request_size_bytes, Some(256));
    assert_eq!(logs[0].response_size_bytes, Some(1024));
    assert!(!logs[0].id.is_empty(), "the stored id is returned");
    assert_ne!(logs[0].id, logs[1].id, "each row has its own id");
    assert_eq!(logs[1].status_code, 200);
    assert_eq!(logs[1].response_time_ms, Some(50));

    let by_status = repo
        .get_request_logs(Some(user_id), None, None, None, Some("5"), None)
        .await
        .unwrap();
    assert_eq!(by_status.len(), 1, "the status filter is a prefix");
    assert_eq!(by_status[0].status_code, 504);

    let by_tool = repo
        .get_request_logs(Some(user_id), None, None, None, None, Some("GET_ACTIV"))
        .await
        .unwrap();
    assert_eq!(
        by_tool.len(),
        1,
        "the tool filter ignores case on both engines"
    );
    assert_eq!(by_tool[0].tool_name, "get_activities");

    let by_key = repo
        .get_request_logs(None, Some(&api_key.id), None, None, None, None)
        .await
        .unwrap();
    assert_eq!(by_key.len(), 2);

    let recent = repo
        .get_request_logs(
            Some(user_id),
            None,
            Some(Utc::now() - Duration::minutes(1)),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(recent.len(), 1, "the window binds as an instant");
    assert_eq!(recent[0].tool_name, "get_athlete");
}

#[tokio::test]
async fn the_key_window_stats_and_top_tools_count_the_recorded_calls() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let api_key = fresh_api_key(&repos, user_id).await;
    let repo = &repos.usage;
    // Whole seconds, so the oldest call reads back equal on Postgres, whose
    // TIMESTAMPTZ keeps microseconds, as on SQLite.
    let now = Utc::now().trunc_subsecs(0);
    let oldest_in_window = now - Duration::minutes(2);

    for (tool, status, at) in [
        ("get_activities", 200, oldest_in_window),
        ("get_activities", 200, now - Duration::minutes(1)),
        ("get_athlete", 500, now),
        ("get_athlete", 200, now - Duration::hours(2)),
    ] {
        repo.record_api_key(&api_call(&api_key.id, tool, status, at))
            .await
            .unwrap();
    }

    let window_start = now - Duration::seconds(i64::from(api_key.rate_limit_window_seconds));
    assert_eq!(
        repo.get_api_key_window_usage(&api_key.id, window_start)
            .await
            .unwrap(),
        ApiKeyWindowUsage {
            count: 3,
            oldest: Some(oldest_in_window),
        },
        "the call two hours ago is outside the key's one-hour window, and the oldest inside it frees the first slot"
    );
    let unused_key = fresh_api_key(&repos, user_id).await;
    assert_eq!(
        repo.get_api_key_window_usage(&unused_key.id, window_start)
            .await
            .unwrap(),
        ApiKeyWindowUsage {
            count: 0,
            oldest: None,
        },
        "a key with no calls has an empty window"
    );

    let stats = repo
        .get_api_key_stats(
            &api_key.id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(stats.total_requests, 3);
    assert_eq!(stats.successful_requests, 2);
    assert_eq!(stats.failed_requests, 1);
    assert_eq!(stats.total_response_time_ms, 150);
    assert_eq!(stats.tool_usage["get_activities"]["count"], 2);
    assert_eq!(stats.tool_usage["get_activities"]["success_count"], 2);
    assert_eq!(stats.tool_usage["get_athlete"]["count"], 1);
    assert_eq!(stats.tool_usage["get_athlete"]["success_count"], 0);
    assert!(
        (stats.tool_usage["get_athlete"]["avg_response_time_ms"]
            .as_f64()
            .unwrap()
            - 50.0)
            .abs()
            < 1e-9
    );

    let tools = repo
        .get_top_tools_analysis(
            user_id,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].tool_name, "get_activities", "most called first");
    assert_eq!(tools[0].request_count, 2);
    assert!((tools[0].success_rate - 100.0).abs() < 1e-9);
    assert!((tools[0].average_response_time - 50.0).abs() < 1e-9);
    assert_eq!(tools[1].tool_name, "get_athlete");
    assert_eq!(tools[1].request_count, 1);
    assert!(
        (tools[1].success_rate).abs() < 1e-9,
        "a 500 is not a success"
    );
}

#[tokio::test]
async fn jwt_usage_counts_this_utc_month_and_not_last_month() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let repo = &repos.usage;

    let now = Utc::now();
    let month_start = now
        .with_day(1)
        .unwrap()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    let last_month = month_start - Duration::hours(1);

    repo.record_jwt_usage(&jwt_call(user_id, now))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(user_id, month_start))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(user_id, last_month))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(fresh_user(&repos).await, now))
        .await
        .unwrap();

    assert_eq!(
        repo.get_jwt_current_usage(user_id).await.unwrap(),
        2,
        "the month's first instant counts, the hour before it does not, another user's call never does"
    );
}

#[tokio::test]
async fn jwt_usage_today_counts_from_the_utc_midnight() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user_id = fresh_user(&repos).await;
    let repo = &repos.usage;

    let now = Utc::now();
    let midnight = utc_day_start(now);
    assert_eq!(
        midnight,
        now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
    );
    assert_eq!(next_utc_day_start(now), midnight + Duration::days(1));

    repo.record_jwt_usage(&jwt_call(user_id, now))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(user_id, midnight))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(user_id, midnight - Duration::seconds(1)))
        .await
        .unwrap();
    repo.record_jwt_usage(&jwt_call(fresh_user(&repos).await, now))
        .await
        .unwrap();

    assert_eq!(
        repo.get_jwt_usage_today(user_id).await.unwrap(),
        2,
        "midnight counts, the second before it does not, another user's call never does"
    );
}

#[tokio::test]
async fn a_counter_increment_returns_its_own_value_with_an_rfc3339_timestamp() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.usage_counters;
    let tenant = Uuid::new_v4().to_string();

    let first = repo
        .increment_counter(&tenant, "user-1", "messages", "2026-09", 3)
        .await
        .unwrap();
    assert_eq!(first.value, 3);
    let stamped = DateTime::parse_from_rfc3339(&first.updated_at)
        .unwrap_or_else(|e| panic!("updated_at {:?} is RFC 3339: {e}", first.updated_at));
    assert!(
        (Utc::now() - stamped.with_timezone(&Utc))
            .num_seconds()
            .abs()
            < 60
    );

    let second = repo
        .increment_counter(&tenant, "user-1", "messages", "2026-09", 4)
        .await
        .unwrap();
    assert_eq!(second.value, 7, "the upsert returns the value it produced");
    assert!(second.updated_at >= first.updated_at);

    let read = repo
        .get_counter(&tenant, "user-1", "messages", "2026-09")
        .await
        .unwrap();
    assert_eq!(read.value, 7);
    assert_eq!(
        read.updated_at, second.updated_at,
        "the read renders the stored stamp the same way"
    );
    assert_eq!(read.period, "2026-09");

    repo.increment_counter(&tenant, "user-1", "messages", "2026-08", 1)
        .await
        .unwrap();
    assert_eq!(
        repo.delete_old_counters("2026-09").await.unwrap(),
        1,
        "only the older bucket of this tenant is pruned here"
    );
    assert_eq!(
        repo.get_counter(&tenant, "user-1", "messages", "2026-08")
            .await
            .unwrap()
            .value,
        0
    );
}

fn llm_call<'a>(
    tenant_id: &'a str,
    user_id: &'a str,
    turn: ConversationTurnId,
    seq: i64,
) -> InsertLlmUsage<'a> {
    InsertLlmUsage {
        tenant_id,
        user_id,
        conversation_id: Some("conv-1"),
        turn_id: turn,
        provider: "gemini",
        model: "gemini-2.5-flash",
        prompt_tokens: 100,
        completion_tokens: 40,
        total_tokens: 140,
        cached_tokens: 10,
        cached_write_tokens: 5,
        reasoning_tokens: 2,
        call_type: "chat",
        tool_calls_count: 1,
        tools_called: "[\"get_activities\"]",
        execution_time_ms: Some(320),
        cost_usd: 0.0021,
        call_sequence: Some(seq),
    }
}

#[tokio::test]
async fn an_llm_usage_row_reads_back_as_its_insert_returned_it() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.llm_usage;
    let tenant = TenantId::generate();
    let tenant_str = tenant.to_string();
    let user = Uuid::new_v4().to_string();
    let turn = ConversationTurnId::from_uuid(Uuid::new_v4());

    let second = repo
        .insert_llm_usage(&llm_call(&tenant_str, &user, turn, 2))
        .await
        .unwrap();
    let first = repo
        .insert_llm_usage(&llm_call(&tenant_str, &user, turn, 1))
        .await
        .unwrap();

    let by_turn = repo.find_llm_usage_by_turn_id(turn).await.unwrap();
    assert_eq!(by_turn.len(), 2);
    assert_eq!(
        by_turn[0].id, first.id,
        "ordered by call_sequence, not insertion"
    );
    assert_eq!(by_turn[0].call_sequence, Some(1));
    assert_eq!(by_turn[1].call_sequence, Some(2));
    assert_eq!(by_turn[0].turn_id, turn);
    assert_eq!(by_turn[0].conversation_id.as_deref(), Some("conv-1"));
    assert_eq!(by_turn[0].tools_called, "[\"get_activities\"]");
    assert_eq!(by_turn[0].cached_write_tokens, 5);
    assert_eq!(by_turn[0].reasoning_tokens, 2);
    assert_eq!(by_turn[0].execution_time_ms, Some(320));
    assert!((by_turn[0].cost_usd - 0.0021).abs() < 1e-12);
    assert_eq!(
        by_turn[0].created_at, first.created_at,
        "the stored instant renders exactly as the insert returned it"
    );
    assert_eq!(by_turn[1].created_at, second.created_at);

    let recent = repo.get_recent_llm_calls_admin(1).await.unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].id, first.id, "newest first");

    let tool_calls = repo
        .get_tenant_tool_calls_since(tenant, "2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert_eq!(tool_calls.len(), 2);
    let silent = InsertLlmUsage {
        tools_called: "[]",
        tool_calls_count: 0,
        ..llm_call(
            &tenant_str,
            &user,
            ConversationTurnId::from_uuid(Uuid::new_v4()),
            1,
        )
    };
    repo.insert_llm_usage(&silent).await.unwrap();
    assert_eq!(
        repo.get_tenant_tool_calls_since(tenant, "2020-01-01T00:00:00+00:00")
            .await
            .unwrap()
            .len(),
        2,
        "a call that invoked no tool stays out of the tool view"
    );

    let (calls, tokens) = repo
        .sum_llm_usage_since("2020-01-01T00:00:00+00:00")
        .await
        .unwrap();
    assert!(calls >= 3, "cross-tenant, at least this test's rows");
    assert!(tokens >= 420);
    assert!(
        repo.count_llm_calls_since("2020-01-01T00:00:00+00:00")
            .await
            .unwrap()
            >= 3
    );

    let spend = repo
        .sum_cost_usd_for_tenant_period(
            tenant,
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::hours(1),
        )
        .await
        .unwrap();
    assert!(
        (spend - 0.0063).abs() < 1e-9,
        "three calls' cost, got {spend}"
    );
}

#[tokio::test]
async fn since_is_a_day_or_an_instant_on_both_engines() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.llm_usage;
    let tenant = Uuid::new_v4().to_string();
    let user = Uuid::new_v4().to_string();
    let turn = ConversationTurnId::from_uuid(Uuid::new_v4());
    repo.insert_llm_usage(&llm_call(&tenant, &user, turn, 1))
        .await
        .unwrap();

    let today = Utc::now().format("%Y-%m-%d").to_string();
    let daily = repo
        .get_llm_usage_daily_series(&tenant, &today)
        .await
        .unwrap();
    assert_eq!(daily.len(), 1, "a bare day is midnight UTC of that day");
    assert_eq!(daily[0].date, today, "grouped by the UTC calendar day");
    assert_eq!(daily[0].tokens, 140);
    assert_eq!(daily[0].calls, 1);
    assert!((daily[0].avg_execution_time_ms - 320.0).abs() < 1e-9);

    let by_user = repo
        .get_llm_usage_daily_series_by_user(&user, "2020-01-01")
        .await
        .unwrap();
    assert_eq!(by_user.len(), 1);
    assert_eq!(by_user[0].cached_write_tokens, 5);

    let aggregates = repo
        .get_llm_usage_aggregates(&tenant, &(Utc::now() - Duration::minutes(1)).to_rfc3339())
        .await
        .unwrap();
    assert_eq!(aggregates.len(), 1);
    assert_eq!(aggregates[0].total_tokens, 140);
    assert_eq!(aggregates[0].reasoning_tokens, 2);
    assert_eq!(aggregates[0].calls, 1);
    assert!(
        repo.get_llm_usage_aggregates_by_user(
            &user,
            &(Utc::now() + Duration::minutes(1)).to_rfc3339()
        )
        .await
        .unwrap()
        .is_empty(),
        "an instant after the row excludes it"
    );

    for garbage in ["yesterday", "2026-09-14 00:00:00", ""] {
        let refused = repo.get_llm_usage_aggregates(&tenant, garbage).await;
        assert_eq!(
            refused.as_ref().err().map(|e| e.code),
            Some(ErrorCode::InvalidInput),
            "since {garbage:?} is refused on both engines, got {refused:?}"
        );
    }
}
