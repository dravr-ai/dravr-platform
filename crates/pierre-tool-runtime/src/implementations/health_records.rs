// ABOUTME: get_health_records — the athlete's lab results, clinical records, glucose and cycle tracking, each row by its own tier
// ABOUTME: Free-text-stripped copies only; outside clients are refused at dispatch (a T2 stream), a coach reaches T2 under their grant only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_health_records` (owl phase 5, slice 2; carnet#1544).
//!
//! The one reader of the T2–T3 records every import stores
//! ([`pierre_services::health_records::read`]). It declares owl's streams
//! `labs.summary` and `clinical.raw` (T2) and `restricted.clinical.summary`
//! and `restricted.selfreport.summary` (T3), so the dispatch tier gate refuses
//! every outside client (a T2 stream is never readable there until grants
//! carry authorization details, slice 3). Within Dravr:
//!
//! - every record is served as its free-text-stripped copy, never the
//!   resource as read, and only once read as it is or confirmed by the
//!   athlete;
//! - each row is read by its own tier under the athlete's consents, every one
//!   left out tallied for the withheld note;
//! - a coach's read reaches T2 only while the athlete's health-records grant
//!   to them stands, and T3 never.

use std::sync::Arc;

use async_trait::async_trait;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{RecordWindow, MAX_HEALTH_RECORDS_PAGE};
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_services::health_records::read::{
    CoachReach, HealthRecordsPage, RecordKind, RecordView, RecordsRead,
};
use pierre_tools_core::ToolResult;
use serde::Serialize;
use serde_json::{json, Value};

use crate::annotations::{annotations, Effect, Reach};
use crate::context::{scoped_read_subject, ToolExecutionContext};
use crate::conversions::{
    answers_with, apply_format, ok_typed, tool_definition, tool_result_to_response, Formatted,
};
use crate::implementations::data_helpers::parse_output_format;
use crate::implementations::stored_data::{date_range_properties, parse_date_range};
use crate::read_subject;
use crate::runtime::ToolRuntime;

/// How many records a call returns when it names no limit.
const DEFAULT_LIMIT: u32 = 50;

/// What `get_health_records` answers with.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct HealthRecordsResult {
    /// The kind read.
    pub kind: RecordKind,
    /// The records of that kind this read reaches.
    #[serde(flatten)]
    pub page: HealthRecordsPage,
    /// The window read, RFC 3339.
    pub start: String,
    /// Its end, RFC 3339.
    pub end: String,
}

fn kind_property() -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(
            "Which records: labs (lab results), clinical (conditions, medications and other \
             clinical records), glucose (self-measured readings and each day's value, mg/dL) or \
             self_reports (cycle tracking). One of: labs, clinical, glucose, self_reports."
                .to_owned(),
        ),
        ..Default::default()
    }
}

fn limit_property() -> PropertySchema {
    PropertySchema {
        property_type: "integer".to_owned(),
        description: Some(format!(
            "At most this many records, 1 to {MAX_HEALTH_RECORDS_PAGE} (default {DEFAULT_LIMIT})"
        )),
        ..Default::default()
    }
}

/// The `kind` a call names.
fn kind_of(args: &Value) -> AppResult<RecordKind> {
    args.get("kind")
        .and_then(Value::as_str)
        .and_then(RecordKind::parse)
        .ok_or_else(|| {
            AppError::invalid_input("kind must be labs, clinical, glucose or self_reports")
        })
}

/// The page size a call names, within bounds.
fn limit_of(args: &Value) -> u32 {
    args.get("limit")
        .and_then(Value::as_u64)
        .and_then(|limit| u32::try_from(limit).ok())
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(1, MAX_HEALTH_RECORDS_PAGE)
}

/// Tool reading the athlete's health records.
#[derive(Debug)]
pub struct GetHealthRecordsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetHealthRecordsTool {
    fn definition(&self) -> Tool {
        let mut properties = date_range_properties();
        properties.insert("kind".to_owned(), kind_property());
        properties.insert("limit".to_owned(), limit_property());
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["kind".to_owned()]),
            ..Default::default()
        };
        answers_with::<Formatted<HealthRecordsResult>>(tool_definition(
            "get_health_records",
            "Get the athlete's health records: lab results, clinical records, self-measured \
             glucose or cycle tracking, as far as they allowed AI to read them",
            schema,
            annotations("Get health records", Effect::ReadOnly, Reach::Internal),
        ))
    }

    /// No provider connection is needed: a lab report the athlete uploaded is
    /// as much theirs as an import.
    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let format = parse_output_format(&args);
            let tenant_id = TenantId::from_uuid(context.require_tenant()?);
            let kind = kind_of(&args)?;
            let (start, end) = parse_date_range(&args)?;
            let coach = scoped_read_subject().map(|_| CoachReach {
                health_records: read_subject::health_records_named(),
            });
            let terms = context.resources.provider_registry();
            let read = RecordsRead {
                repos: context.resources.repos(),
                terms: terms.as_ref(),
                user_id: context.user_id,
                tenant: &tenant_id,
                view: RecordView::Served,
                coach,
                project: read_subject::readable_wellness(),
            };
            let window = RecordWindow {
                since: Some(start),
                until: Some(end),
                limit: limit_of(&args),
            };
            match Box::pin(read.read(kind, window)).await {
                Ok(page) => ok_typed(
                    "get_health_records",
                    apply_format(
                        HealthRecordsResult {
                            kind,
                            page,
                            start: start.to_rfc3339(),
                            end: end.to_rfc3339(),
                        },
                        format,
                    ),
                ),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to read health records: {e}"),
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

crate::declare_security!(GetHealthRecordsTool => empty);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_names_one_kind_of_four() {
        assert_eq!(
            kind_of(&json!({ "kind": "labs" })).ok(),
            Some(RecordKind::Labs),
            "bug: labs"
        );
        assert!(
            kind_of(&json!({ "kind": "notes" })).is_err(),
            "bug: an unknown kind"
        );
        assert!(kind_of(&json!({})).is_err(), "bug: no kind");
    }

    #[test]
    fn the_page_is_kept_within_bounds() {
        assert_eq!(limit_of(&json!({})), DEFAULT_LIMIT, "bug: the default");
        assert_eq!(limit_of(&json!({ "limit": 0 })), 1, "bug: an empty page");
        assert_eq!(
            limit_of(&json!({ "limit": 10_000 })),
            MAX_HEALTH_RECORDS_PAGE,
            "bug: the cap"
        );
    }
}
