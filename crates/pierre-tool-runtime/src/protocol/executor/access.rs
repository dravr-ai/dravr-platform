// ABOUTME: The access refusal at the dispatch chokepoint: what the one access predicate withholds, rendered per answer
// ABOUTME: Grant, transport, role and support session refuse a call exactly as its caller's listing leaves it out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_barrage::Stream;
use pierre_auth::oauth2_server::cimd::host_profile_tier;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_contremaitre::consent_strings::step_up_worded;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::{grant_ceiling_refusal, UniversalExecutor};
use crate::first_party_tools;
use crate::protocol::types::UniversalResponse;
use crate::protocols::ProtocolError;
use crate::security::{Audience, RuntimeTool};
use crate::tool_access::{
    coach_role_refusal, is_human_coach, withheld_call, Caller, CallerRole, Withheld,
};

impl UniversalExecutor {
    /// What [`withheld_call`] answers for this call with `args`, rendered
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
        (tool_name, args): (&str, &Value),
        user_id: Uuid,
        is_admin: bool,
        ceiling: Option<&GrantCeiling>,
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
            ceiling,
        };
        let registry = self.resources.tool_registry();
        let answer = withheld_call(registry, tool_name, tool, &caller, args).or_else(|| {
            if self.caller_writes_reply {
                return None;
            }
            let role_less = Caller {
                role: CallerRole::default(),
                ..caller
            };
            withheld_call(registry, tool_name, tool, &role_less, args)
                .filter(|why| *why != Withheld::Role(Audience::Admin))
        });
        // A tier or stream refusal the athlete's step-up would cure says so,
        // once the step-up screen is worded (carnet#1544 slice 4b), to a
        // client that can complete one, as `/mcp`'s 403 challenge is given.
        if let (Some(Withheld::Tier(_) | Withheld::Stream), Some(ceiling)) = (answer, ceiling) {
            let streams: Vec<Stream> = registry
                .call_data(tool_name, args)
                .map_or_default(|data| data.required_streams().collect());
            if ceiling.step_up_cures(&streams)
                && step_up_worded(self.resources.messaging_strings_registry())
                && self.client_can_step_up().await
            {
                return Ok(Some(grant_ceiling_refusal::step_up(tool_name, &streams)));
            }
        }
        answer
            .map(|answer| render_withheld(tool_name, answer))
            .transpose()
    }

    /// Whether the calling client can complete a step-up: its metadata
    /// document declares a Host Profile T2, which every step-up approval
    /// needs. A client registered by DCR, or whose profile cannot be read,
    /// cannot, so it is never told to step up.
    async fn client_can_step_up(&self) -> bool {
        let Some(client_id) = self
            .credential
            .as_ref()
            .and_then(|credential| credential.client_id.as_deref())
        else {
            return false;
        };
        self.resources
            .repos()
            .oauth2_server
            .client_host_profile(client_id)
            .await
            .inspect_err(|e| warn!(client_id, "a client's Host Profile could not be read: {e}"))
            .ok()
            .flatten()
            .as_deref()
            .and_then(host_profile_tier)
            .is_some_and(|tier| tier.eq_ignore_ascii_case("T2"))
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
/// - A tool beyond the consent grant's ceiling (its tier, its streams, its
///   window): an in-band refusal naming why ([`grant_ceiling_refusal`]);
///   no scope would lift it, only another grant.
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
        Withheld::Tier(tier) => Ok(grant_ceiling_refusal::tier(tool_name, tier)),
        Withheld::Stream => Ok(grant_ceiling_refusal::stream(tool_name)),
        Withheld::Window => Ok(grant_ceiling_refusal::window(tool_name)),
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
