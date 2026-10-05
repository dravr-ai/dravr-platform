// ABOUTME: The derived content an activity's own thread carries, held to the terms of the provider it is about
// ABOUTME: Rows carry their own provenance stamp; a thread opened from an activity carries it on the link to that activity

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Derived content over an external transport (carnet#769).
//!
//! Content derived from an athlete's data is stamped where it is written: a
//! reply, a fact, a note, a plan and the rest carry the
//! [`TransportPolicy`] of what they were built from, and a reader withholds a
//! stamped row from an external caller — in its SQL where it lists, with
//! [`ai_scope::admit_derived`](pierre_providers::ai_scope::admit_derived)
//! where it reads one.
//!
//! One kind of content carries its provenance on a link rather than on its
//! rows: a conversation opened from an activity's view is about that activity
//! (`activity_conversations`). Its first question names the activity, its
//! title may too, and every turn in it reasons over it — so the link is
//! stamped with the activity's terms when it is written, and the whole thread
//! is held to them.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_providers::ai_scope;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// The stamp a thread carries through its activity links: the strictest of
/// them, [`TransportPolicy::AnyTransport`] for a thread no activity opened.
///
/// # Errors
///
/// The repository error when the links cannot be read.
pub async fn thread_policy(
    runtime: &dyn ToolRuntime,
    tenant_id: &TenantId,
    user_id: Uuid,
    conversation_id: &str,
) -> AppResult<TransportPolicy> {
    let links = runtime
        .repos()
        .activity_conversations
        .list_activity_conversation_links(tenant_id, user_id)
        .await?;
    Ok(TransportPolicy::strictest_of(
        links
            .iter()
            .filter(|link| link.conversation_id == conversation_id)
            .map(|link| link.transport_policy),
    ))
}

/// Refuse an external caller a thread opened from a first-party-only
/// activity; a no-op on Dravr's own surfaces.
///
/// # Errors
///
/// [`ErrorCode::UnavailableOverTransport`](pierre_core::errors::ErrorCode::UnavailableOverTransport)
/// when the thread is withheld here, and the repository error when the links
/// cannot be read.
pub async fn refuse_withheld_thread(
    runtime: &dyn ToolRuntime,
    tenant_id: &TenantId,
    user_id: Uuid,
    conversation_id: &str,
) -> AppResult<()> {
    if !ai_scope::serving_external() {
        return Ok(());
    }
    let policy = thread_policy(runtime, tenant_id, user_id, conversation_id).await?;
    if policy.is_first_party_only() {
        return Err(AppError::unavailable_over_transport(
            "this conversation is not available over this interface",
        ));
    }
    Ok(())
}
