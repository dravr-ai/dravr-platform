// ABOUTME: MCP tool handlers for querying persisted health data from repositories
// ABOUTME: Implements get_sleep_sessions, get_recovery_metrics, get_health_snapshots, list_data_sources
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::future::Future;
use std::pin::Pin;

use chrono::{Duration, Utc};
use serde_json::json;

use crate::models::TenantId;
use crate::protocols::universal::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use crate::protocols::ProtocolError;
use crate::utils::uuid::parse_user_id_for_protocol;

use super::{apply_format_to_response, extract_output_format};

/// Default number of days to look back when no date range is specified
const DEFAULT_LOOKBACK_DAYS: i64 = 30;

/// Parse start/end date parameters from the request, defaulting to last 30 days
#[must_use]
fn parse_date_range(request: &UniversalRequest) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let end = request
        .parameters
        .get("end")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(Utc::now, |dt| dt.with_timezone(&Utc));

    let start = request
        .parameters
        .get("start")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map_or_else(
            || Utc::now() - Duration::days(DEFAULT_LOOKBACK_DAYS),
            |dt| dt.with_timezone(&Utc),
        );

    (start, end)
}

/// Extract tenant ID from the request, falling back to `user_id`
fn extract_tenant_id(request: &UniversalRequest) -> Result<TenantId, ProtocolError> {
    let user_id_string = request.user_id.clone();
    request
        .tenant_id
        .as_deref()
        .unwrap_or(&user_id_string)
        .parse()
        .map_err(|_| ProtocolError::InvalidRequest("Invalid tenant_id format".to_owned()))
}

/// Handle `get_sleep_sessions` tool — query stored sleep sessions from the database
#[must_use]
pub fn handle_get_sleep_sessions(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let format = extract_output_format(&request);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let tenant_id = extract_tenant_id(&request)?;
        let (start, end) = parse_date_range(&request);

        match executor
            .resources
            .repos
            .sleep
            .get_sleep_sessions(user_uuid, &tenant_id, start, end)
            .await
        {
            Ok(sessions) => {
                let result = json!({
                    "count": sessions.len(),
                    "sessions": sessions,
                    "range": {
                        "start": start.to_rfc3339(),
                        "end": end.to_rfc3339(),
                    }
                });
                Ok(apply_format_to_response(
                    UniversalResponse {
                        success: true,
                        result: Some(result),
                        error: None,
                        metadata: None,
                    },
                    "result",
                    format,
                ))
            }
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch sleep sessions: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `get_recovery_metrics` tool — query stored recovery and readiness data
#[must_use]
pub fn handle_get_recovery_metrics(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let format = extract_output_format(&request);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let tenant_id = extract_tenant_id(&request)?;
        let (start, end) = parse_date_range(&request);

        match executor
            .resources
            .repos
            .recovery
            .get_recovery_metrics(user_uuid, &tenant_id, start, end)
            .await
        {
            Ok(metrics) => {
                let result = json!({
                    "count": metrics.len(),
                    "metrics": metrics,
                    "range": {
                        "start": start.to_rfc3339(),
                        "end": end.to_rfc3339(),
                    }
                });
                Ok(apply_format_to_response(
                    UniversalResponse {
                        success: true,
                        result: Some(result),
                        error: None,
                        metadata: None,
                    },
                    "result",
                    format,
                ))
            }
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch recovery metrics: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `get_health_snapshots` tool — query stored body composition and vitals
#[must_use]
pub fn handle_get_health_snapshots(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let format = extract_output_format(&request);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let tenant_id = extract_tenant_id(&request)?;
        let (start, end) = parse_date_range(&request);

        match executor
            .resources
            .repos
            .health_snapshots
            .get_health_snapshots(user_uuid, &tenant_id, start, end)
            .await
        {
            Ok(snapshots) => {
                let result = json!({
                    "count": snapshots.len(),
                    "snapshots": snapshots,
                    "range": {
                        "start": start.to_rfc3339(),
                        "end": end.to_rfc3339(),
                    }
                });
                Ok(apply_format_to_response(
                    UniversalResponse {
                        success: true,
                        result: Some(result),
                        error: None,
                        metadata: None,
                    },
                    "result",
                    format,
                ))
            }
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to fetch health snapshots: {e}")),
                metadata: None,
            }),
        }
    })
}

/// Handle `list_data_sources` tool — list connected devices and providers
#[must_use]
pub fn handle_list_data_sources(
    executor: &UniversalToolExecutor,
    request: UniversalRequest,
) -> Pin<Box<dyn Future<Output = Result<UniversalResponse, ProtocolError>> + Send + '_>> {
    Box::pin(async move {
        let format = extract_output_format(&request);
        let user_uuid = parse_user_id_for_protocol(&request.user_id)?;
        let tenant_id = extract_tenant_id(&request)?;

        match executor
            .resources
            .repos
            .data_sources
            .list_data_sources(user_uuid, &tenant_id)
            .await
        {
            Ok(sources) => {
                let result = json!({
                    "count": sources.len(),
                    "sources": sources,
                });
                Ok(apply_format_to_response(
                    UniversalResponse {
                        success: true,
                        result: Some(result),
                        error: None,
                        metadata: None,
                    },
                    "result",
                    format,
                ))
            }
            Err(e) => Ok(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Failed to list data sources: {e}")),
                metadata: None,
            }),
        }
    })
}
