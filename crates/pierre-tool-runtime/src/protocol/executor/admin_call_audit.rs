// ABOUTME: The admin audit trail for admin-only tool calls from outside Dravr's own surfaces (MCP, A2A, API key)
// ABOUTME: One audit_events row per call, written before the call is dispatched and settled to its outcome; no row, no call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin-only tool calls on the audit trail
//!
//! An admin's API key carries every scope, so an outside client holding one
//! can run the admin-only tools, five of which write: they create, rewrite,
//! delete and assign the agents and system prompts that answer users. The
//! REST admin routes write each action to `audit_events`, and each such tool
//! call over `/mcp` or A2A is one row of the same trail, naming the caller,
//! the tool, the transport and how the call ended (EFVP 2026-11-17, decision
//! 9.4d).
//!
//! The row is written **before** the call is dispatched, as
//! [`AuditOutcome::Started`], and settled afterwards. A row that cannot be
//! written refuses the call, so no admin-only call from outside goes
//! unrecorded; one that cannot be settled stays `started`, which is what the
//! trail then says of it. Every admin-only call is recorded, reads included,
//! and whoever makes it: a caller the admin gate refuses shows as `refused`.
//! A call the `/mcp` or A2A door refuses before it reaches the executor (no
//! tenant, a quota, a tool switched off for the tenant) is written `refused`
//! by that door through [`record_refused_admin_call`], so every admin-only
//! call over those transports is on the trail.
//!
//! **In an admin's support session** ([`crate::support_session`],
//! carnet#1511) every call from outside is recorded the same way, whatever
//! the tool: the row's actor is the admin, and its metadata names the user
//! the session acts as and the session. A row that cannot be written
//! refuses the call, a plain read included.
//!
//! Calls from Dravr's own surfaces (the web and mobile apps, messaging) are
//! not recorded here: the admin's own session already answers for them. A
//! call that declared no transport is recorded, as the AI gates treat it as
//! external (`ai_scope::result_exposure`); its row names the transport
//! [`UNDECLARED_TRANSPORT`](pierre_core::transport::UNDECLARED_TRANSPORT).

use pierre_auth::auth::ImpersonationGrant;
use pierre_core::transport::Transport;
use pierre_database::repositories::{
    AdminAuditRepository, AuditOutcome, AuditSource, NewAuditEvent, SettledAuditEvent,
};
use serde_json::json;
use tracing::{error, warn};
use uuid::Uuid;

use super::UniversalExecutor;
use crate::protocol::types::{UniversalRequest, UniversalResponse};
use crate::protocols::ProtocolError;
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_providers::ai_scope;

/// The `action` of a tool call's row: `tool.` and the tool's name.
const ACTION_PREFIX: &str = "tool.";

/// The refusal of a call whose row could not be written.
const UNRECORDED: &str =
    "its record on the admin audit trail could not be written, so it was not run";

/// A started row, to settle once the call has ended.
#[derive(Debug)]
pub(super) struct StartedAdminCall {
    id: String,
    actor: Option<Uuid>,
    tenant: Option<String>,
    tool_name: String,
}

impl UniversalExecutor {
    /// The started row of `request` when it is an admin-only tool call this
    /// executor serves from outside Dravr's own surfaces; `None` for any
    /// other call, which the trail does not record here.
    ///
    /// # Errors
    ///
    /// Returns the refusal the call answers with when its row cannot be
    /// written.
    pub(super) async fn admin_call_audit(
        &self,
        request: &UniversalRequest,
    ) -> Result<Option<StartedAdminCall>, ProtocolError> {
        let transport = ai_scope::effective_transport(self.transport);
        let support = self.support_session.as_ref();
        let recorded =
            support.is_some() || is_admin_only(self.resources.as_ref(), &request.tool_name);
        if !recorded || !recorded_over(transport) {
            return Ok(None);
        }
        start(
            self.resources.repos().admin_audit.as_ref(),
            request,
            transport,
            support,
        )
        .await
        .map(Some)
    }
}

/// Whether `tool_name` is a registered admin-only tool.
fn is_admin_only(runtime: &dyn ToolRuntime, tool_name: &str) -> bool {
    runtime
        .tool_registry()
        .get(tool_name)
        .is_some_and(|tool| tool.capabilities().contains(ToolCapabilities::ADMIN_ONLY))
}

/// Whether a call of an admin-only tool served over `transport` is recorded:
/// it reaches the tool from outside Dravr's own surfaces, or it declared no
/// transport, which is read as outside.
#[must_use]
fn recorded_over(transport: Option<Transport>) -> bool {
    transport.is_none_or(|served| !served.is_first_party())
}

/// The row of an admin-only tool call, before it is written.
fn event_of(
    tool_name: &str,
    protocol: &str,
    actor: Option<Uuid>,
    tenant_id: Option<String>,
    transport: Option<Transport>,
    outcome: AuditOutcome,
) -> NewAuditEvent {
    let label = Transport::label_of(transport);
    NewAuditEvent {
        action: format!("{ACTION_PREFIX}{tool_name}"),
        summary: format!("{tool_name} called over {label}"),
        outcome,
        source: AuditSource::ToolCall,
        actor_user_id: actor,
        tenant_id,
        ip_address: None,
        user_agent: None,
        metadata: json!({
            "tool": tool_name,
            "transport": label,
            "protocol": protocol,
        }),
    }
}

/// An admin-only tool call a door refused before it reached the executor.
#[derive(Debug, Clone, Copy)]
pub struct RefusedAdminCall<'call> {
    /// The tool the caller named.
    pub tool_name: &'call str,
    /// The caller.
    pub user_id: Uuid,
    /// The tenant the call acted in, when the door resolved one.
    pub tenant_id: Option<&'call str>,
    /// The transport the door serves.
    pub transport: Transport,
    /// The protocol the call came in on (`mcp`, `a2a`).
    pub protocol: &'call str,
}

/// Write the `refused` row of `call`, an admin-only tool call a door refused.
///
/// Only an admin-only tool over a recorded transport writes one; any other
/// call writes nothing. The call was refused, so nothing ran: a row that
/// cannot be written is logged.
pub async fn record_refused_admin_call(runtime: &dyn ToolRuntime, call: &RefusedAdminCall<'_>) {
    let transport = Some(call.transport);
    if !is_admin_only(runtime, call.tool_name) || !recorded_over(transport) {
        return;
    }
    let event = event_of(
        call.tool_name,
        call.protocol,
        Some(call.user_id),
        call.tenant_id.map(str::to_owned),
        transport,
        AuditOutcome::Refused,
    );
    if let Err(e) = runtime.repos().admin_audit.record(&event).await {
        error!(
            tool_name = %call.tool_name,
            transport = Transport::label_of(transport),
            "admin-only tool call refused before dispatch; its audit row could not be written: {e}"
        );
    }
}

/// Write the started row of `request`, an admin-only tool call served over
/// `transport` (`None`: undeclared).
///
/// # Errors
///
/// Returns the refusal the call answers with when the row cannot be written.
async fn start(
    trail: &dyn AdminAuditRepository,
    request: &UniversalRequest,
    transport: Option<Transport>,
    support: Option<&ImpersonationGrant>,
) -> Result<StartedAdminCall, ProtocolError> {
    // In a support session the admin acted, as the user.
    let actor = support.map_or_else(
        || Uuid::parse_str(&request.user_id).ok(),
        |grant| Some(grant.impersonator_id),
    );
    let label = Transport::label_of(transport);
    let mut event = event_of(
        &request.tool_name,
        &request.protocol,
        actor,
        request.tenant_id.clone(),
        transport,
        AuditOutcome::Started,
    );
    if let Some(grant) = support {
        event.summary = format!(
            "{} called over {label} in a support session",
            request.tool_name
        );
        event.metadata["target"] = json!({ "user_id": request.user_id });
        event.metadata["impersonation_session_id"] = json!(grant.session_id);
    }
    match trail.record(&event).await {
        Ok(id) => Ok(StartedAdminCall {
            id,
            actor,
            tenant: request.tenant_id.clone(),
            tool_name: request.tool_name.clone(),
        }),
        Err(e) => {
            error!(
                tool_name = %request.tool_name,
                transport = label,
                "admin-only tool call refused: its audit row could not be written: {e}"
            );
            Err(ProtocolError::ExecutionFailed(format!(
                "'{}' was refused: {UNRECORDED}",
                request.tool_name
            )))
        }
    }
}

/// How a dispatched call ended, as its row records it.
fn outcome_of(result: &Result<UniversalResponse, ProtocolError>) -> AuditOutcome {
    let failed = match result {
        Ok(response) if response.success => return AuditOutcome::Success,
        Ok(_) => false,
        Err(error) => matches!(
            error,
            ProtocolError::InternalError(_)
                | ProtocolError::ExecutionFailed(_)
                | ProtocolError::ExecutionFailedDetailed { .. }
                | ProtocolError::Database { .. }
                | ProtocolError::Serialization { .. }
                | ProtocolError::SerializationError(_)
                | ProtocolError::SerializationErrorDetailed { .. }
                | ProtocolError::ConversionFailed { .. }
                | ProtocolError::ConfigMissing { .. }
                | ProtocolError::ConfigurationError(_)
                | ProtocolError::ConfigurationErrorDetailed { .. }
        ),
    };
    if failed {
        AuditOutcome::Failed
    } else {
        AuditOutcome::Refused
    }
}

/// Settle `started` to how `result` ended. The call has already run, so a
/// row that cannot be settled is logged and left `started`.
pub(super) async fn settle(
    trail: &dyn AdminAuditRepository,
    started: &StartedAdminCall,
    result: &Result<UniversalResponse, ProtocolError>,
) {
    let outcome = outcome_of(result);
    let settled = SettledAuditEvent {
        id: &started.id,
        actor_user_id: started.actor,
        tenant_id: started.tenant.as_deref(),
        outcome,
    };
    match trail.settle(&settled).await {
        Ok(true) => {}
        Ok(false) => warn!(
            tool_name = %started.tool_name,
            audit_event_id = %started.id,
            "admin-only tool call's audit row was no longer started; left as it was"
        ),
        Err(e) => error!(
            tool_name = %started.tool_name,
            audit_event_id = %started.id,
            outcome = outcome.as_str(),
            "admin-only tool call's audit row could not be settled; it stays started: {e}"
        ),
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::transport::UNDECLARED_TRANSPORT;

    use super::*;

    #[test]
    fn only_a_call_from_outside_dravrs_own_surfaces_is_recorded() {
        for transport in [Transport::McpHttp, Transport::A2a, Transport::ApiKey] {
            assert!(recorded_over(Some(transport)), "{transport:?}");
        }
        for transport in [
            Transport::WebApp,
            Transport::MobileApp,
            Transport::Messaging,
            Transport::PlatformJob,
        ] {
            assert!(!recorded_over(Some(transport)), "{transport:?}");
        }
        assert!(
            recorded_over(None),
            "bug: a call that declared no transport is read as outside, and recorded"
        );
        assert_eq!(
            Transport::label_of(None),
            UNDECLARED_TRANSPORT,
            "bug: an undeclared call is labelled undeclared"
        );
    }

    #[test]
    fn a_call_ends_as_success_refused_or_failed() {
        let answered = |success| {
            Ok(UniversalResponse {
                success,
                result: None,
                error: None,
                metadata: None,
            })
        };
        assert_eq!(
            outcome_of(&answered(true)),
            AuditOutcome::Success,
            "bug: a successful answer settles as success"
        );
        assert_eq!(
            outcome_of(&answered(false)),
            AuditOutcome::Refused,
            "bug: an error answer settles as refused"
        );
        assert_eq!(
            outcome_of(&Err(ProtocolError::PermissionDenied {
                tool_name: "admin_delete_system_agent".to_owned(),
                reason: "Admin access required".to_owned(),
            })),
            AuditOutcome::Refused,
            "bug: a permission refusal settles as refused"
        );
        assert_eq!(
            outcome_of(&Err(ProtocolError::InternalError("boom".to_owned()))),
            AuditOutcome::Failed,
            "bug: a server fault settles as failed"
        );
    }
}
