// ABOUTME: Numeric accuracy tests for the admin Usage Analytics time series
// ABOUTME: Pins the call-weighted latency mean and the today-inclusive day window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The admin Usage Analytics tab renders `get_usage_analytics`. These tests
//! pin the two figures whose value depends on how the day window is built:
//! the daily request series and the average response time.
//!
//! `llm_usage` rows are seeded through `SeederRepository::seed_insert_llm_usage`
//! because it is the only write path that accepts an explicit `created_at`,
//! which is what makes a multi-day window testable at all.

#![allow(missing_docs, clippy::unwrap_used)]

mod common;

use anyhow::Result;
use chrono::{Duration, NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, init_server_config};
use pierre_auth::auth::{AuthMethod, AuthResult};
use pierre_auth::rate_limiting::UnifiedRateLimitInfo;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::seed_models::SeedLlmUsageRecord;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_dashboard::service::DashboardRoutes;
use std::sync::Arc;
use uuid::Uuid;

/// Tolerance in milliseconds for comparing a computed mean against a
/// hand-computed one.
const TOLERANCE_MS: f64 = 1e-9;

/// A day of seeded LLM activity: `days_ago` back from today (UTC), `calls`
/// rows each reporting `execution_time_ms`.
struct SeededDay {
    days_ago: i64,
    calls: usize,
    execution_time_ms: i64,
}

/// Activity every test in this file seeds, spread over a 5-day window with
/// two idle days in the middle.
///
/// Hand-computed call-weighted mean:
///   (1 x 100 + 3 x 200 + 4 x 50) / (1 + 3 + 4) = 900 / 8 = 112.5 ms
///
/// The unweighted mean of daily means over a 5-day zero-padded window is
/// (100 + 0 + 200 + 0 + 50) / 5 = 70.0 ms, and over a window that stops at
/// yesterday it is (0 + 100 + 0 + 200 + 0) / 5 = 60.0 ms — both far enough
/// from 112.5 that either arrangement fails these assertions.
const SEEDED_DAYS: [SeededDay; 3] = [
    SeededDay {
        days_ago: 4,
        calls: 1,
        execution_time_ms: 100,
    },
    SeededDay {
        days_ago: 2,
        calls: 3,
        execution_time_ms: 200,
    },
    SeededDay {
        days_ago: 0,
        calls: 4,
        execution_time_ms: 50,
    },
];

/// Expected call-weighted average response time for [`SEEDED_DAYS`].
const EXPECTED_WEIGHTED_MEAN_MS: f64 = 112.5;

/// Total calls seeded by [`SEEDED_DAYS`].
const EXPECTED_TOTAL_CALLS: u64 = 8;

struct AnalyticsFixture {
    routes: DashboardRoutes<ServerContext>,
    user_id: Uuid,
    tenant_id: Uuid,
    today: NaiveDate,
}

impl AnalyticsFixture {
    /// Build a tenant, seed [`SEEDED_DAYS`] into `llm_usage`, and hand back a
    /// dashboard service bound to the same runtime context.
    async fn new(email: &str) -> Result<Self> {
        init_server_config();
        let resources = create_test_server_resources().await?;
        let (user_id, _user, tenant) =
            create_test_user_with_plan(&resources.coach.database, email, "starter").await?;
        let tenant_id = tenant.as_uuid();
        let today = Utc::now().date_naive();

        for day in &SEEDED_DAYS {
            // Midday keeps the row unambiguously inside its calendar day no
            // matter what hour the suite runs at.
            let created_at = (today - Duration::days(day.days_ago))
                .and_hms_opt(12, 0, 0)
                .ok_or_else(|| anyhow::Error::msg("failed to build seeded timestamp"))?
                .and_utc();

            for _ in 0..day.calls {
                let record = SeedLlmUsageRecord {
                    id: Uuid::new_v4(),
                    tenant_id,
                    user_id,
                    conversation_id: None,
                    provider: "google".to_owned(),
                    model: "gemini-2.0-flash".to_owned(),
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                    call_type: "chat".to_owned(),
                    tool_calls_count: 0,
                    execution_time_ms: day.execution_time_ms,
                    created_at,
                };
                resources
                    .common
                    .repos
                    .seeder
                    .seed_insert_llm_usage(&record)
                    .await?;
            }
        }

        Ok(Self {
            routes: DashboardRoutes::new(Arc::clone(&resources)),
            user_id,
            tenant_id,
            today,
        })
    }

    fn auth(&self) -> AuthResult {
        AuthResult {
            scopes: OAuthScope::self_grant(),
            session_id: None,
            user_id: self.user_id,
            auth_method: AuthMethod::JwtToken {
                tier: "premium".to_owned(),
            },
            rate_limit: UnifiedRateLimitInfo {
                is_rate_limited: false,
                limit: Some(1000),
                remaining: Some(1000),
                reset_at: None,
                tier: "premium".to_owned(),
                auth_method: "jwt".to_owned(),
            },
            active_tenant_id: Some(self.tenant_id),
        }
    }

    fn day_label(&self, days_ago: i64) -> String {
        (self.today - Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }
}

/// The five-day window ends on today, and every seeded day lands on the
/// calendar day it was written for.
#[tokio::test]
async fn window_ends_on_today_and_aligns_each_day() -> Result<()> {
    let fixture = AnalyticsFixture::new("analytics-window@example.com").await?;
    let analytics = fixture
        .routes
        .get_usage_analytics(fixture.auth(), 5)
        .await?;

    assert_eq!(
        analytics.time_series.len(),
        5,
        "a 5-day request must emit exactly 5 points"
    );

    let dates: Vec<&str> = analytics
        .time_series
        .iter()
        .map(|p| p.date.as_str())
        .collect();
    let expected_dates: Vec<String> = (0..5).rev().map(|d| fixture.day_label(d)).collect();
    assert_eq!(
        dates, expected_dates,
        "window must run from today-4 through today inclusive"
    );

    assert_eq!(
        analytics
            .time_series
            .last()
            .map(|p| p.date.clone())
            .unwrap(),
        fixture.day_label(0),
        "the final point must be the current UTC day"
    );

    let counts: Vec<u64> = analytics
        .time_series
        .iter()
        .map(|p| p.request_count)
        .collect();
    assert_eq!(
        counts,
        vec![1u64, 0, 3, 0, 4],
        "seeded calls must land on their own day, today included"
    );

    Ok(())
}

/// The average response time is the call-weighted mean of the days that
/// recorded calls, not the arithmetic mean of a zero-padded window.
#[tokio::test]
async fn average_response_time_is_call_weighted() -> Result<()> {
    let fixture = AnalyticsFixture::new("analytics-weighted@example.com").await?;
    let analytics = fixture
        .routes
        .get_usage_analytics(fixture.auth(), 5)
        .await?;

    let total: u64 = analytics.time_series.iter().map(|p| p.request_count).sum();
    assert_eq!(
        total, EXPECTED_TOTAL_CALLS,
        "all seeded calls must be inside the window"
    );

    assert!(
        (analytics.average_response_time - EXPECTED_WEIGHTED_MEAN_MS).abs() < TOLERANCE_MS,
        "expected the call-weighted mean {EXPECTED_WEIGHTED_MEAN_MS} ms, got {}",
        analytics.average_response_time
    );

    Ok(())
}

/// Widening the range adds only idle days, so the reported latency must not
/// move. This is the operator-visible symptom: the same calls read 1012.75 ms
/// at 30 days and 337.58 ms at 90 days when idle days are averaged in.
#[tokio::test]
async fn average_response_time_is_independent_of_range_width() -> Result<()> {
    let fixture = AnalyticsFixture::new("analytics-range@example.com").await?;

    let narrow = fixture
        .routes
        .get_usage_analytics(fixture.auth(), 7)
        .await?;
    let wide = fixture
        .routes
        .get_usage_analytics(fixture.auth(), 90)
        .await?;

    assert_eq!(narrow.time_series.len(), 7);
    assert_eq!(wide.time_series.len(), 90);

    let narrow_total: u64 = narrow.time_series.iter().map(|p| p.request_count).sum();
    let wide_total: u64 = wide.time_series.iter().map(|p| p.request_count).sum();
    assert_eq!(narrow_total, EXPECTED_TOTAL_CALLS);
    assert_eq!(
        wide_total, EXPECTED_TOTAL_CALLS,
        "the wider range must contain the same calls, not more"
    );

    assert!(
        (narrow.average_response_time - EXPECTED_WEIGHTED_MEAN_MS).abs() < TOLERANCE_MS,
        "7-day mean should be {EXPECTED_WEIGHTED_MEAN_MS} ms, got {}",
        narrow.average_response_time
    );
    assert!(
        (wide.average_response_time - narrow.average_response_time).abs() < TOLERANCE_MS,
        "widening 7 -> 90 days changed latency from {} ms to {} ms with identical calls",
        narrow.average_response_time,
        wide.average_response_time
    );

    Ok(())
}
