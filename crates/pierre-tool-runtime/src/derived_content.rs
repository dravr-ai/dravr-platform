// ABOUTME: Keeps content derived from an athlete's data off external transports when a first-party-only provider fed it
// ABOUTME: Transcripts, memories, notes, plans and playbooks carry no provenance, so the athlete's connections decide

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Derived content over an external transport (carnet#724).
//!
//! A turn's reply, the facts and notes extracted from it, its compaction
//! summaries, a saved plan and a coaching playbook can all be built from a
//! provider's data, and none of them records which provider. Some terms forbid
//! making a provider's data available outside Dravr's own surfaces "including
//! in derived or aggregated form" (Nolio §6.9), so an external caller — an MCP
//! client, an A2A agent, an API key — must not read such content.
//!
//! Without provenance on each item, the rule is decided by the athlete's
//! connections: while the athlete holds a connection to a provider whose terms
//! keep its data first-party, every derived reader refuses an external call.
//! The athlete's own surfaces read everything.

use pierre_core::ai_policy::first_party_only;
use pierre_core::errors::{AppError, AppResult};
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Refuse a read of content derived from the athlete's data when the call is
/// external and the athlete holds a first-party-only provider's connection.
///
/// # Errors
///
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode::UnavailableOverTransport)
/// when the content is not served over this transport, and the repository
/// error when the athlete's connections cannot be read.
pub async fn refuse_derived_content_off_interface(
    runtime: &dyn ToolRuntime,
    user_id: Uuid,
) -> AppResult<()> {
    if !ai_scope::exposure().is_some_and(|gate| gate.external) {
        return Ok(());
    }
    let terms = runtime.provider_registry();
    let connections = runtime
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await?;
    if connections
        .iter()
        .any(|connection| first_party_only(terms.as_ref(), &connection.provider, None))
    {
        return Err(AppError::unavailable_over_transport(
            "this athlete's conversations, notes and plans are not available over this interface",
        ));
    }
    Ok(())
}
