// ABOUTME: System prompt assembly stage — LLM message construction from history
// ABOUTME: Extracted from services/chat_orchestration.rs::build_llm_messages (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! System prompt assembly helpers.
//!
//! The full prompt-building pipeline for a turn is driven by
//! [`super::super::run`], which composes coach/default text,
//! [`super::super::stages::refresh`] freshness hints, Tier 2 memory recall
//! ([`super::memory`]), Tier 4 followups ([`super::followups`]), and the
//! channel-profile response-constraints suffix. This module owns the
//! final mechanical step: turning the assembled system prompt and the
//! prior message history into a flat `Vec<ChatMessage>` for the LLM.

use std::borrow::Cow;
use std::fmt::Write as _;
use std::sync::Arc;

use pierre_database::database::MessageRecord;
use uuid::Uuid;

use crate::llm::ChatMessage;
use crate::mcp::resources::ServerContext;
use crate::models::ConnectionType;

#[cfg(feature = "tools-groups")]
use crate::errors::AppResult;
#[cfg(feature = "tools-groups")]
use crate::models::TenantId;
#[cfg(feature = "tools-groups")]
use crate::services::group_fitness::{fetch_member_snapshots, MemberSnapshot};

/// Resolve group context strictly from the conversation record's
/// `group_id`.
///
/// Group context (member snapshots, group-scoped prompt injection) is
/// opt-in: a conversation must be explicitly created with
/// `group_id = Some(...)` to receive it. 1:1 personal conversations —
/// where `group_id` is `None` — never auto-attach to a group the user
/// happens to belong to. Doing so would leak another group member's
/// fitness data into a private chat and confuse the LLM about whose
/// activities the user is asking about.
///
/// Returns `(Some(group_id), snapshots)` when the conversation is
/// group-scoped and members are found, or `(None, empty_vec)` otherwise.
///
/// # Errors
///
/// Database errors from the group-member lookup are swallowed (the
/// function degrades to an empty member list); this function itself
/// currently only propagates `AppError` for signature symmetry with
/// the rest of the pipeline stages — no variants are produced today.
#[cfg(feature = "tools-groups")]
pub async fn resolve_group_context(
    resources: &Arc<ServerContext>,
    conversation_group_id: Option<&str>,
    tool_tenant_id: TenantId,
) -> AppResult<(Option<String>, Vec<MemberSnapshot>)> {
    let Some(gid) = conversation_group_id else {
        return Ok((None, Vec::new()));
    };

    let member_ids: Vec<uuid::Uuid> = resources
        .repos
        .groups
        .list_members(gid)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.user_id)
        .collect();

    let snapshots = if member_ids.is_empty() {
        Vec::new()
    } else {
        fetch_member_snapshots(resources, &member_ids, tool_tenant_id).await
    };

    Ok((Some(gid.to_owned()), snapshots))
}

/// Build LLM messages from conversation history and optional system prompt.
///
/// The system prompt (when provided) leads the message list; history is
/// appended in order, dropping messages with unknown roles defensively.
#[must_use]
pub fn build_llm_messages(
    system_prompt: Option<&str>,
    history: &[MessageRecord],
) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);

    if let Some(prompt) = system_prompt {
        messages.push(ChatMessage::system(prompt));
    }

    for msg in history {
        let chat_msg = match msg.role.as_str() {
            "user" => ChatMessage::user(&msg.content),
            "assistant" => ChatMessage::assistant(&msg.content),
            "system" => ChatMessage::system(&msg.content),
            _ => continue,
        };
        messages.push(chat_msg);
    }

    messages
}

/// Build the "Connected Fitness Data Providers" system-prompt section.
///
/// Appended so the LLM does not ask users to connect providers that are
/// already connected.
///
/// Uses `provider_connections` as the single source of truth (cross-tenant
/// view) and filters out providers that are not registered in the current
/// runtime (e.g. synthetic providers excluded from production builds).
/// Returns an empty string when the user has no registered connections —
/// callers append this unconditionally.
pub async fn build_provider_context(resources: &Arc<ServerContext>, user_id: Uuid) -> String {
    // Get all provider connections (cross-tenant view, single source of truth)
    let Ok(connections) = resources
        .repos
        .provider_connections
        .get_for_user(user_id, None)
        .await
    else {
        return String::new();
    };

    // Filter out providers that aren't registered in the current runtime
    // (e.g., synthetic providers excluded from production builds)
    let connections: Vec<_> = connections
        .into_iter()
        .filter(|c| resources.provider_registry.is_supported(&c.provider))
        .collect();

    if connections.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n\n## Connected Fitness Data Providers\n\n");
    context.push_str("The user has the following data sources available:\n");
    for conn in &connections {
        let label = if conn.connection_type == ConnectionType::Synthetic {
            Cow::Owned(format!("{} (test data)", conn.provider))
        } else {
            Cow::Borrowed(conn.provider.as_str())
        };
        // Write trait used to avoid format_push_string lint
        let _ = writeln!(context, "- ✓ {label}");
    }
    context.push_str("\nUse the connected providers to fetch activity data. ");
    context.push_str("Do NOT ask the user to connect providers that are already connected above.");

    context
}

/// Build the "Available Tools" section from the runtime tool registry.
///
/// Replaces the previously-static tool list that lived in
/// `pierre_system.md`. Generating it from the registry at assembly time
/// prevents the two from drifting as tools are added, renamed, or
/// removed — a drift that historically led the LLM to invent
/// capabilities (e.g. "look up Uber Eats menus") when the static list
/// stopped reflecting reality. Each user-visible tool gets one line:
/// `` - `name`: description ``. Admin-only tools are excluded.
#[must_use]
pub fn build_tools_section(resources: &Arc<ServerContext>) -> String {
    let schemas = resources.tool_registry.user_visible_schemas();

    let mut out = String::with_capacity(2_048);
    out.push_str("## Available Tools\n\n");
    out.push_str("You have exactly the tools listed below. You do **not** have any tool that is not in this list: ");
    out.push_str(
        "you cannot browse the web, scrape menus, look up prices, use third-party services, ",
    );
    out.push_str(
        "or run arbitrary code. If a request requires a capability not covered here, say so ",
    );
    out.push_str("honestly rather than inventing a plan. Call tools with the parameters described in their schemas.\n\n");

    for schema in schemas {
        // One line per tool: `- `name`: description (first line only)`.
        // Multi-line descriptions exist for a handful of tools; the LLM
        // sees the full schema via native function-calling, so trimming
        // the prompt-side description to the lead sentence keeps the
        // system prompt compact.
        let description_lead = schema.description.lines().next().unwrap_or("").trim();
        let _ = writeln!(out, "- `{}`: {}", schema.name, description_lead);
    }

    out
}
