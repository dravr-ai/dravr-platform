// ABOUTME: Shared helpers for the tools that write to an athlete's provider calendar
// ABOUTME: Builds the authed calendar provider and the destructive-tool annotation set both tool files share
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_mcp_schema::ToolAnnotations;
use pierre_providers::core::FitnessProvider;
use pierre_services::plan_calendar_push::CALENDAR_PROVIDER;
use uuid::Uuid;

use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;

/// Annotation set for a tool that deletes calendar entries on the provider:
/// destructive, but safe to repeat (a second call finds nothing to delete).
pub(super) fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Build the athlete's authenticated calendar provider, or the refusal a
/// calendar-less athlete gets.
pub(super) async fn calendar_provider(
    context: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Box<dyn FitnessProvider>> {
    let tenant_str = tenant_id.to_string();
    AuthService::new(context.resources.clone())
        .create_authenticated_provider(CALENDAR_PROVIDER, user_id, Some(tenant_str.as_str()))
        .await
        .map_err(|resp| {
            AppError::invalid_input(resp.error.unwrap_or_else(|| {
                "Connect an Intervals.icu account first — it is the only calendar this \
                 platform can write to"
                    .to_owned()
            }))
        })
}
