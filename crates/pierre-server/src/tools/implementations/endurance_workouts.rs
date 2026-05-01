// ABOUTME: Endurance Phase 5 MCP tools — list_workout_templates + prescribe_workout
// ABOUTME: Surfaces the six cornerstone workouts and pushes a prescription to the prescribed_workouts audit table
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{PrescribedWorkout, TenantId};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::services::workout_library::{cornerstone_templates, require_cornerstone};
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        open_world_hint: Some(false),
        ..ToolAnnotations::default()
    }
}

fn write_safe_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(false),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

fn require_tenant(context: &ToolExecutionContext) -> AppResult<TenantId> {
    context.tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
        AppError::new(
            ErrorCode::AuthInvalid,
            "Endurance workout tools require an active tenant context",
        )
    })
}

/// `list_workout_templates` — read-only catalog of cornerstones.
pub struct ListWorkoutTemplatesTool;

#[async_trait]
impl McpTool for ListWorkoutTemplatesTool {
    fn name(&self) -> &'static str {
        "list_workout_templates"
    }

    fn description(&self) -> &'static str {
        "List the Endurance cornerstone workout templates: long_run_z2, \
         threshold_4x8, vo2_5x3, recovery_30min, tempo_progression, \
         sweet_spot_2x20. Each row carries the structured steps + target \
         zones the prescribe_workout tool will push to the user's \
         Intervals.icu calendar."
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: Some(Vec::new()),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, _context: &ToolExecutionContext) -> AppResult<ToolResult> {
        drop(args);
        let templates = cornerstone_templates();
        let payload = serde_json::to_value(&templates)
            .map_err(|e| AppError::internal(format!("serialize templates: {e}")))?;
        Ok(ToolResult::ok(json!({
            "count": templates.len(),
            "templates": payload,
        })))
    }
}

/// `prescribe_workout` — record a prescription audit row + (queued) push to Intervals.icu.
pub struct PrescribeWorkoutTool;

#[async_trait]
impl McpTool for PrescribeWorkoutTool {
    fn name(&self) -> &'static str {
        "prescribe_workout"
    }

    fn description(&self) -> &'static str {
        "Prescribe one of the six Endurance cornerstone workouts to a \
         specific calendar date for the authenticated user. Records the \
         prescription in the prescribed_workouts audit trail and queues \
         it for push to Intervals.icu. Args: template_slug (one of \
         long_run_z2, threshold_4x8, vo2_5x3, recovery_30min, \
         tempo_progression, sweet_spot_2x20), date (YYYY-MM-DD), optional \
         coach_id."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "template_slug".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Slug of the cornerstone template to prescribe.".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "date".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Calendar date the workout is scheduled for (YYYY-MM-DD).".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Optional coach id stamped onto the audit row.".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["template_slug".to_owned(), "date".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::REQUIRES_PROVIDER
            | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_safe_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        let tenant_id = require_tenant(context)?;
        let user_id = context.user_id;
        let slug = args
            .get("template_slug")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("template_slug is required"))?;
        let date_str = args
            .get("date")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::invalid_input("date is required"))?;
        let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map_err(|e| AppError::invalid_input(format!("date must be YYYY-MM-DD: {e}")))?;
        let coach_id = args
            .get("coach_id")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let template = require_cornerstone(slug)?;
        let payload_json = serde_json::to_string(&template)
            .map_err(|e| AppError::internal(format!("serialize template: {e}")))?;

        let prescription_id = Uuid::new_v4();
        let prescribed = PrescribedWorkout {
            id: prescription_id,
            tenant_id: tenant_id.as_uuid(),
            user_id,
            coach_id,
            template_slug: template.slug.clone(),
            sport: template.sport.clone(),
            prescribed_for_date: date,
            provider: "intervals_icu".to_owned(),
            provider_event_id: None,
            payload_json,
            status: "queued".to_owned(),
            created_at: Utc::now(),
        };
        context
            .resources
            .repos
            .prescribed_workouts
            .upsert_prescribed_workout(&prescribed)
            .await?;
        Ok(ToolResult::ok(json!({
            "prescription_id": prescription_id,
            "provider": "intervals_icu",
            "provider_event_id": Value::Null,
            "template_slug": template.slug,
            "scheduled_for": date.format("%Y-%m-%d").to_string(),
            "status": "queued",
        })))
    }
}

/// Build the Endurance workout-tool list for registry registration.
#[must_use]
pub fn create_endurance_workout_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ListWorkoutTemplatesTool),
        Box::new(PrescribeWorkoutTool),
    ]
}
