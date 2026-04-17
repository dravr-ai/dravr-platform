// ABOUTME: DataRequirements-driven activity prefetch stage — deterministically loads activities before LLM dispatch
// ABOUTME: Extracted from routes/chat.rs::ChatRoutes prefetch_activity_context / inject_startup_context / get_startup_context_if_applicable (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `DataRequirements` activity prefetch.
//!
//! When the conversation's coach has a `data_requirements.activities` block
//! in its YAML frontmatter, this stage deterministically invokes
//! `get_activities` with exact parameters from the coach definition before
//! the LLM runs. The fetched activities are formatted and injected as a
//! system-prompt section ("The following activity data has been pre-loaded
//! for your analysis"), and the coach's startup query (if any) is appended
//! as the analysis instruction.
//!
//! Before this stage was extended to messaging, only web chat's first-turn
//! dispatch ran prefetch. On Telegram / `WhatsApp` / Discord / Slack the coach
//! had to decide to call `get_activities` itself — and when the LLM skipped
//! the call, it would hallucinate activity details (observed 2026-04-16 on
//! a Telegram conversation about route planning between Prévost and
//! Saint-Alexis-des-Monts, where the coach admitted "je n'ai pas encore
//! chargé les détails exacts de cette sortie").
//!
//! Applicability gating still follows the original web-chat rule — prefetch
//! only runs on the first message of a coach-attached conversation. Extending
//! that to later turns (e.g. based on user-input heuristics) is a follow-up.

use std::sync::Arc;

use pierre_core::models::coaches::DataRequirements;
use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::MessageRecord;
use tracing::{info, warn};

use crate::llm::ChatMessage;
use crate::models::TenantId;
use crate::protocols::universal::{UniversalExecutor, UniversalResponse};

/// Extract the formatted content string from a `get_activities` response.
///
/// Handles both string results and JSON object results by serializing
/// the object to a compact string representation suitable for LLM context.
fn extract_prefetch_content(response: &UniversalResponse) -> String {
    match &response.result {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

/// Decide whether the turn should run the activity prefetch.
///
/// Returns `Some((query, data_requirements))` when:
/// - This is the first message in the conversation (`history_len == 1`)
/// - The conversation has a resolved coach context
/// - The coach has a `startup_query` or `data_requirements` configured
///
/// Returns `None` otherwise.
#[must_use]
pub fn get_startup_context_if_applicable(
    history_len: usize,
    coach_ctx: Option<&CoachRuntimeContext>,
) -> Option<(Option<String>, Option<DataRequirements>)> {
    if history_len != 1 {
        return None;
    }

    let ctx = coach_ctx?;

    let query = ctx.startup_query.clone();
    let data_reqs = ctx.data_requirements.as_ref().and_then(|json| {
        match serde_json::from_str::<DataRequirements>(json) {
            Ok(dr) => {
                info!("Found data_requirements for coach context assembly");
                Some(dr)
            }
            Err(e) => {
                warn!("Failed to parse data_requirements JSON: {e}");
                None
            }
        }
    });

    if query.is_none() && data_reqs.is_none() {
        return None;
    }

    if let Some(q) = &query {
        info!(
            "Found startup query for coach conversation: {}",
            &q[..q.len().min(50)]
        );
    }

    Some((query, data_reqs))
}

/// Pre-fetch activity data based on structured `DataRequirements`.
///
/// Calls `get_activities` deterministically with exact parameters from the
/// coach definition, bypassing LLM interpretation. Returns the activity
/// data as a formatted string for injection into the conversation context,
/// or `None` when the coach has no activity requirements or the tool call
/// fails.
pub async fn prefetch_activity_context(
    executor: &Arc<UniversalExecutor>,
    user_id: &str,
    tenant_id: TenantId,
    data_reqs: &DataRequirements,
) -> Option<String> {
    use crate::protocols::universal::handlers::fitness_api::handle_get_activities;
    use crate::protocols::universal::UniversalRequest;

    let activities_req = data_reqs.activities.as_ref()?;

    // Build parameters JSON matching what handle_get_activities expects
    let mut params = serde_json::json!({
        "limit": activities_req.count,
        "mode": activities_req.mode,
        "format": activities_req.format,
        "analysis_type": activities_req.analysis_type,
    });

    // Add sport_type filter if specified (single sport type for now)
    if let Some(sport_type) = activities_req.sport_types.first() {
        params["sport_type"] = serde_json::Value::String(String::clone(sport_type));
    }

    // Add time_frame as 'after' timestamp
    if let Some(seconds) = activities_req.time_frame_seconds() {
        let after = chrono::Utc::now().timestamp() - seconds;
        params["after"] = serde_json::Value::Number(serde_json::Number::from(after));
    }

    let request = UniversalRequest {
        tool_name: "get_activities".to_owned(),
        parameters: params,
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match handle_get_activities(executor, request).await {
        Ok(response) => {
            let content = extract_prefetch_content(&response);
            info!(
                content_len = content.len(),
                "Pre-fetched activity context for coach"
            );
            Some(content)
        }
        Err(e) => {
            warn!("Failed to pre-fetch activity context: {e}");
            None
        }
    }
}

/// Inject startup context (pre-fetched data + analysis query) into LLM messages.
///
/// When a coach has `data_requirements`, activity data is fetched
/// deterministically via [`prefetch_activity_context`] and injected as
/// system context immediately after the system prompt. The coach's
/// startup query becomes the analysis instruction appended before the
/// user turn.
///
/// Without `data_requirements`, the startup query is injected as-is for
/// the LLM to interpret (tool-calling fallback).
pub async fn inject_startup_context(
    executor: &Arc<UniversalExecutor>,
    llm_messages: &mut Vec<ChatMessage>,
    history: &[MessageRecord],
    coach_ctx: Option<&CoachRuntimeContext>,
    user_id: &str,
    tenant_id: TenantId,
) {
    let Some((startup_query, data_reqs)) =
        get_startup_context_if_applicable(history.len(), coach_ctx)
    else {
        return;
    };

    if let Some(data_reqs) = &data_reqs {
        // Full context assembly: pre-fetch activity data deterministically
        if let Some(activity_context) =
            prefetch_activity_context(executor, user_id, tenant_id, data_reqs).await
        {
            let context_msg = format!(
                "The following activity data has been pre-loaded for your analysis:\n\n\
                 {activity_context}"
            );
            llm_messages.insert(1, ChatMessage::system(&context_msg));
        }

        // Inject the startup query as the analysis instruction (after data context)
        if let Some(query) = &startup_query {
            let insert_pos = llm_messages.len().saturating_sub(1);
            llm_messages.insert(insert_pos, ChatMessage::user(query));
        }
    } else if let Some(query) = &startup_query {
        // No data_requirements: inject startup query for LLM tool-calling
        llm_messages.insert(1, ChatMessage::user(query));
    }
}
