// ABOUTME: Provider-agnostic sleep fetch helpers reused by analytics + sleep tools
// ABOUTME: Lifted from pierre-server's sleep tool when analytics moved into pierre-tool-runtime
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Sleep data fetch helpers.
//!
//! Analytics (training_load, fitness_score) needs the most recent night of
//! sleep data when scoring recovery. The original implementation lived in
//! `pierre-server::tools::implementations::sleep::inner`, but analytics moved
//! into pierre-tool-runtime — and pierre-tool-runtime cannot depend on
//! pierre-server. The two functions analytics actually needs
//! (`fetch_provider_sleep_data`, `convert_sleep_session_to_data`) are
//! provider-agnostic and depend only on `ToolRuntime` + `UniversalToolExecutor`,
//! so they live here. The remaining sleep-tool helpers stay in pierre-server
//! and import the converter from this module.

use crate::protocol::types::{UniversalResponse, UniversalToolExecutor};
use chrono::{Duration, Utc};
use pierre_core::models::{SleepSession, SleepStageType};
use pierre_intelligence::SleepData;
use tracing::warn;
use uuid::Uuid;

/// Provider-agnostic sleep data fetcher
///
/// Fetches sleep data from any provider that supports sleep tracking (Fitbit, Garmin, WHOOP, Terra).
/// Uses `AuthService` for tenant-aware credential lookup and provider instantiation.
/// Automatically converts provider-specific `SleepSession` to the unified `SleepData` format.
///
/// # Arguments
/// * `executor` - The tool executor with access to auth service and provider registry
/// * `user_uuid` - The user's UUID for token lookup
/// * `tenant_id` - Optional tenant ID for multi-tenant deployments
/// * `provider_name` - Name of the sleep-capable provider
/// * `days_back` - Number of days of sleep data to fetch (default: 1 for most recent night)
///
/// # Errors
/// Returns `UniversalResponse` with error if provider doesn't support sleep or fetch fails
pub async fn fetch_provider_sleep_data(
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
    provider_name: &str,
    days_back: u32,
) -> Result<SleepData, UniversalResponse> {
    // Check if provider supports sleep tracking
    let capabilities = executor
        .resources
        .provider_registry()
        .get_capabilities(provider_name)
        .ok_or_else(|| UniversalResponse {
            success: false,
            result: None,
            error: Some(format!("Provider '{provider_name}' not found in registry")),
            metadata: None,
        })?;

    if !capabilities.supports_sleep() {
        return Err(UniversalResponse {
            success: false,
            result: None,
            error: Some(format!(
                "Provider '{provider_name}' does not support sleep tracking. \
                 Use a sleep-capable provider: fitbit, garmin, whoop, or terra."
            )),
            metadata: None,
        });
    }

    // Use AuthService for tenant-aware authenticated provider creation
    let provider = executor
        .auth_service
        .create_authenticated_provider(provider_name, user_uuid, tenant_id)
        .await?;

    // Fetch sleep sessions with a wider query window to account for providers
    // (like WHOOP) that may index sleep by cycle boundary rather than start time.
    // A 1-day request uses a 3-day API window; multi-day requests add 2 extra days.
    let query_days = i64::from(days_back) + 2;
    let end_date = Utc::now();
    let start_date = end_date - Duration::days(query_days);

    let sessions = provider
        .get_sleep_sessions(start_date, end_date)
        .await
        .map_err(|e| {
            warn!(
                provider = provider_name,
                error = %e,
                "Failed to fetch sleep data from provider"
            );
            UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "Sleep data is not available from {provider_name} right now. \
                     The device may not have synced recent data yet."
                )),
                metadata: None,
            }
        })?;

    // Get most recent session and convert to SleepData
    let session = sessions
        .into_iter()
        .next()
        .ok_or_else(|| UniversalResponse {
            success: false,
            result: None,
            error: Some(format!(
                "No sleep data available from '{provider_name}' for the last {days_back} day(s)"
            )),
            metadata: None,
        })?;

    Ok(convert_sleep_session_to_data(&session))
}

/// Convert a provider `SleepSession` to the intelligence layer `SleepData` format
#[must_use]
pub fn convert_sleep_session_to_data(session: &SleepSession) -> SleepData {
    // Calculate stage durations from sleep stages
    let mut deep_minutes: u32 = 0;
    let mut rem_minutes: u32 = 0;
    let mut light_minutes: u32 = 0;
    let mut awake_minutes: u32 = 0;

    for stage in &session.stages {
        match stage.stage_type {
            SleepStageType::Deep => deep_minutes += stage.duration_minutes,
            SleepStageType::Rem => rem_minutes += stage.duration_minutes,
            SleepStageType::Light => light_minutes += stage.duration_minutes,
            SleepStageType::Awake => awake_minutes += stage.duration_minutes,
        }
    }

    // Convert minutes to hours
    let minutes_to_hours = |m: u32| -> Option<f64> {
        if m > 0 {
            Some(f64::from(m) / 60.0)
        } else {
            None
        }
    };

    SleepData {
        date: session.start_time,
        duration_hours: f64::from(session.total_sleep_time) / 60.0,
        deep_sleep_hours: minutes_to_hours(deep_minutes),
        rem_sleep_hours: minutes_to_hours(rem_minutes),
        light_sleep_hours: minutes_to_hours(light_minutes),
        awake_hours: minutes_to_hours(awake_minutes),
        efficiency_percent: Some(f64::from(session.sleep_efficiency)),
        hrv_rmssd_ms: session.hrv_during_sleep,
        resting_hr_bpm: None, // SleepSession doesn't include this directly
        provider_score: session.sleep_score.map(f64::from),
    }
}
