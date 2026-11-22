// ABOUTME: The access refusal at the dispatch chokepoint: what the one access predicate withholds, rendered per answer
// ABOUTME: Grant, transport, role and support session refuse a call exactly as its caller's listing leaves it out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use tracing::warn;
use uuid::Uuid;

use super::UniversalExecutor;
use crate::first_party_tools;
use crate::protocol::types::UniversalResponse;
use crate::protocols::ProtocolError;
use crate::security::{Audience, RuntimeTool};
use crate::tool_access::{
    coach_role_refusal, is_human_coach, withheld, Caller, CallerRole, Withheld,
};

impl UniversalExecutor {
    /// What [`withheld`] answers for this call, rendered
    /// ([`render_withheld`]): `None` when the call may run.
    ///
    /// `is_admin` is the live role
    /// [`build_tool_context`](super::subject::build_tool_context) read. The
    /// coach flag is read live too, and only where it can change the answer: a coach
    /// tool called by an MCP client or an A2A agent (the entry points that
    /// bind [`Self::with_caller_writing_reply`]) over an external transport,
    /// by someone who is not an admin. In Dravr's own app the coach tools'
    /// own gates decide.
    ///
    /// A chat turn served outside the app (an API key's, the A2A text turn's)
    /// offers what a caller with no role is offered
    /// ([`offered_schemas`](crate::chat_catalogue::offered_schemas)), so its
    /// calls are refused the same, the admin tools' own gate aside.
    pub(super) async fn access_refusal(
        &self,
        tool: &dyn RuntimeTool,
        tool_name: &str,
        user_id: Uuid,
        is_admin: bool,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        let transport = ai_scope::effective_transport(self.transport);
        let external = !transport.is_some_and(Transport::is_first_party);
        let coach = self.caller_writes_reply
            && external
            && !is_admin
            && tool.audience() == Audience::Coach
            && is_human_coach(self.resources.repos(), user_id).await;
        let caller = Caller {
            role: CallerRole {
                admin: is_admin,
                coach,
            },
            grant: &self.scopes,
            transport,
            support_session: self.support_session.is_some(),
        };
        let answer = withheld(tool, &caller).or_else(|| {
            if self.caller_writes_reply {
                return None;
            }
            let role_less = Caller {
                role: CallerRole::default(),
                ..caller
            };
            withheld(tool, &role_less).filter(|why| *why != Withheld::Role(Audience::Admin))
        });
        answer
            .map(|answer| render_withheld(tool_name, answer))
            .transpose()
    }
}

/// The refusal of an export in an admin's support session: support may see
/// what it needs to help, never carry the whole record away (as the REST
/// export refuses it, `data_export.rs`).
pub const SUPPORT_SESSION_EXPORT_REFUSED: &str =
    "An admin acting as this account cannot export its record; only the account's owner can";

/// The refusal [`withheld`]'s answer renders at the dispatch chokepoint.
///
/// - The grant does not cover the tool: a [`ProtocolError`], because this is
///   the transport-agnostic chokepoint every transport funnels through. The
///   HTTP transport renders the same answer as RFC 6750's 403 +
///   `insufficient_scope` challenge before dispatch, reading the same
///   derivation, and a caller who slipped past the transport is still refused
///   here.
/// - The transport keeps it in Dravr's own app: the in-band
///   [`first_party_tools::refusal`], the answer its absence from the caller's
///   listing gives.
/// - An admin tool for a non-admin: the `PermissionDenied` a tool body's own
///   `require_admin` refusal is re-raised as, so every transport sees one
///   authz shape. Denies by default when the admin lookup failed.
/// - A coach tool for an external caller who is neither a human coach nor an
///   admin: the in-band [`coach_role_refusal`], never a scope challenge.
fn render_withheld(
    tool_name: &str,
    withheld: Withheld,
) -> Result<UniversalResponse, ProtocolError> {
    match withheld {
        Withheld::Scope(missing) => {
            warn!(
                tool_name = %tool_name,
                missing_scope = %missing,
                "tool denied: the caller's grant does not cover it"
            );
            Err(ProtocolError::PermissionDenied {
                tool_name: tool_name.to_owned(),
                reason: format!("the '{missing}' scope is required"),
            })
        }
        Withheld::Transport => Ok(first_party_tools::refusal(tool_name)),
        Withheld::Role(Audience::Admin) => {
            warn!(
                tool_name = %tool_name,
                "admin-only tool denied for non-admin caller at the dispatch chokepoint"
            );
            Err(ProtocolError::PermissionDenied {
                tool_name: tool_name.to_owned(),
                reason: "Admin access required".to_owned(),
            })
        }
        Withheld::Role(Audience::Coach | Audience::Athlete) => Ok(coach_role_refusal(tool_name)),
        Withheld::SupportSession => {
            warn!(
                tool_name = %tool_name,
                "export denied in an admin's support session at the dispatch chokepoint"
            );
            Err(ProtocolError::PermissionDenied {
                tool_name: tool_name.to_owned(),
                reason: SUPPORT_SESSION_EXPORT_REFUSED.to_owned(),
            })
        }
    }
}
