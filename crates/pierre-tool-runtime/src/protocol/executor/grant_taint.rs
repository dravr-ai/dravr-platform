// ABOUTME: The grant-scoped member-read taint of /mcp and A2A tool calls, which carry no topic to hold one
// ABOUTME: A grant that read another member has its free-text writes refused for thirty minutes after the read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The grant-scoped taint (carnet#1511 call 30).
//!
//! A chat turn's free-text writes are decided by the reads its topic
//! carries (call 24). A tool call over `/mcp` or A2A carries no topic: one
//! call reads a member, the next writes free text the caller's later
//! prompts read. The taint is the grant's instead: after a call by a grant
//! that read another member, a free-text write by the same grant is refused
//! until [`GRANT_TAINT_MINUTES`] have passed since that read, and the
//! refusal names the wait.
//!
//! Dispatch alone decides it, and `tools/list` and the A2A extended card
//! still list the free-text writes inside the window: the second recorded
//! exception to I7 beside the loopback's list-once (call 25). `/mcp` holds no
//! session, so no `notifications/tools/list_changed` reaches a client when
//! the window opens or closes, and a client that listed before a read keeps
//! offering what it listed; the refusal names the wait instead.
//!
//! The grant is the credential the call came through: the OAuth grant (the
//! refresh family its token was minted in) or the API key, else the caller's
//! credential of one kind and client. A call that answers a chat turn (it
//! carries a turn token) is decided by its topic and never here.

use std::collections::BTreeSet;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::permissions::impersonation::BoundCredential;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use serde_json::json;
use tracing::warn;
use uuid::Uuid;

use super::UniversalExecutor;
use crate::coach_seat::{self, MEMBER_READ_ERROR_CODE};
use crate::protocol::types::UniversalResponse;
use crate::protocols::ProtocolError;

/// How long after a grant read another member its free-text writes wait.
pub(super) const GRANT_TAINT_MINUTES: i64 = 30;

/// The refusal's `reason`.
const GRANT_MEMBER_READ: &str = "grant_member_read";

impl UniversalExecutor {
    /// The grant this call came through, when the taint decides the call.
    ///
    /// Only a one-shot `/mcp` or A2A call: `None` on Dravr's own surfaces,
    /// the API-key chat route, and a call answering a chat turn.
    fn taint_grant(&self, caller: Uuid) -> Option<String> {
        if self.turn_token.is_some() {
            return None;
        }
        let transport = ai_scope::effective_transport(self.transport)?;
        if !matches!(transport, Transport::McpHttp | Transport::A2a) {
            return None;
        }
        Some(self.credential.as_ref().map_or_else(
            || format!("caller:{caller}"),
            |credential| match credential.binding.as_ref() {
                Some(BoundCredential::Grant(family)) => format!("grant:{family}"),
                Some(BoundCredential::ApiKey(key)) => format!("api_key:{key}"),
                None => format!(
                    "credential:{caller}:{:?}:{}",
                    credential.kind,
                    credential.client_id.as_deref().unwrap_or_default()
                ),
            },
        ))
    }

    /// The refusal of `tool_name`, a free-text write, by a recently tainted
    /// grant.
    ///
    /// Refused while the grant read another member less than
    /// [`GRANT_TAINT_MINUTES`] ago; `None` when it may run.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the taint cannot be read: the write is
    /// refused rather than run.
    pub(super) async fn grant_taint_refusal(
        &self,
        tool_name: &str,
        caller: Uuid,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        // A write that declared no kind is free text, as on every chat seat.
        let free_text = coach_seat::files_free_text(self.resources.tool_registry(), tool_name);
        let Some(grant) = self.taint_grant(caller).filter(|_| free_text) else {
            return Ok(None);
        };
        let Some(read_at) = self
            .resources
            .repos()
            .grant_member_reads
            .grant_member_read_at(caller, &grant)
            .await
            .map_err(|e| {
                warn!(error = %e, "a grant's member read could not be read; the write is refused");
                ProtocolError::InternalError(
                    "whether this connection read another member could not be read".to_owned(),
                )
            })?
        else {
            return Ok(None);
        };
        let ends = read_at + Elapsed::minutes(GRANT_TAINT_MINUTES);
        let now = Timestamp::now();
        if now >= ends {
            return Ok(None);
        }
        let left_ms = ends.epoch_millis() - now.epoch_millis();
        let minutes_left = (left_ms + 59_999) / 60_000;
        Ok(Some(UniversalResponse {
            success: false,
            result: Some(json!({
                "error_code": MEMBER_READ_ERROR_CODE,
                "reason": GRANT_MEMBER_READ,
                "retry_after_minutes": minutes_left,
            })),
            error: Some(format!(
                "'{tool_name}' files free text a later prompt reads, and this connection read \
                 another member's data within the last {GRANT_TAINT_MINUTES} minutes, so nothing \
                 of theirs may be written into this account yet. Wait {minutes_left} minute(s) \
                 and try again."
            )),
            metadata: None,
        }))
    }

    /// Record that this call's grant read a member other than `caller`.
    ///
    /// Only when `members_read` holds one and the call is one the taint
    /// decides. A record that cannot be written is logged: the read was
    /// already served.
    pub(super) async fn note_grant_member_reads(
        &self,
        caller: Uuid,
        members_read: &BTreeSet<Uuid>,
    ) {
        if !members_read.iter().any(|member| *member != caller) {
            return;
        }
        let Some(grant) = self.taint_grant(caller) else {
            return;
        };
        if let Err(e) = self
            .resources
            .repos()
            .grant_member_reads
            .record_grant_member_read(caller, &grant, Timestamp::now())
            .await
        {
            warn!(error = %e, "a grant's read of another member was not recorded");
        }
    }
}
