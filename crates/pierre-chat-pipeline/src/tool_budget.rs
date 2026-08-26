// ABOUTME: Resolves the tool-loop iteration budget for one chat turn
// ABOUTME: Coach column, then admin config, then the compiled-in default — each clamped to the band

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool-loop iteration budget for a turn.
//!
//! Four sources answer, in precedence order: a surface profile that fixes the
//! budget outright, the coach's own `max_tool_iterations` column, the
//! `tool_execution.max_iterations` admin configuration parameter, and finally
//! the compiled-in [`DEFAULT_MAX_TOOL_ITERATIONS`].

use pierre_core::constants::tool_execution::{
    DEFAULT_MAX_TOOL_ITERATIONS, MAX_MAX_TOOL_ITERATIONS, MIN_MAX_TOOL_ITERATIONS,
};
use pierre_core::models::CoachRuntimeContext;
use pierre_runtime_context::ConfigLookupScope;
use tracing::debug;

use crate::surface_profile::TurnBudget;
use crate::ChatPipelineContext;

/// Hold a stored budget inside the supported band.
///
/// Both sources are range-checked where they are written — the coach column by
/// the coach create/update route, the admin parameter by the admin config
/// service's `valid_range` — so this is the read-side floor that keeps a row
/// predating those checks, or edited straight in the database, from handing the
/// loop a zero or a runaway ceiling.
fn clamp_tool_iterations(raw: i64) -> usize {
    let clamped = raw.clamp(
        i64::from(MIN_MAX_TOOL_ITERATIONS),
        i64::from(MAX_MAX_TOOL_ITERATIONS),
    );
    usize::try_from(clamped).unwrap_or_else(|_| usize::from(DEFAULT_MAX_TOOL_ITERATIONS))
}

/// Resolve the tool-loop iteration budget for the turn.
pub async fn resolve_max_iterations(
    policy: TurnBudget,
    ctx: &ChatPipelineContext,
    coach_ctx: Option<&CoachRuntimeContext>,
) -> usize {
    if let TurnBudget::Fixed(n) = policy {
        return n;
    }

    if let Some(iterations) = coach_ctx.and_then(|c| c.max_tool_iterations) {
        let value = clamp_tool_iterations(i64::from(iterations));
        debug!(
            max_tool_iterations = value,
            "Using coach-level tool iteration limit"
        );
        return value;
    }

    if let Some(ref admin_config) = ctx.admin_config {
        if let Ok(Some(val)) = admin_config
            .get_value("tool_execution.max_iterations", ConfigLookupScope::global())
            .await
        {
            if let Some(config_val) = val.as_i64() {
                let value = clamp_tool_iterations(config_val);
                debug!(
                    max_tool_iterations = value,
                    "Using admin config tool iteration limit"
                );
                return value;
            }
        }
    }

    usize::from(DEFAULT_MAX_TOOL_ITERATIONS)
}
