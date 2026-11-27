// ABOUTME: The access refusal at the dispatch chokepoint: what the one access predicate withholds, rendered per answer
// ABOUTME: Grant, transport, role and support session refuse a call exactly as its caller's listing leaves it out

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_barrage::Stream;
use dravr_tronc::mcp::tool::ToolCapabilities;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_auth::oauth2_server::host_profile::{self, CurableCall};
use pierre_contremaitre::consent_strings::step_up_worded;
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use serde_json::Value;
use tracing::warn;
use uuid::Uuid;

use super::{grant_ceiling_refusal, UniversalExecutor};
use crate::coach_seat;
use crate::first_party_tools;
use crate::protocol::types::UniversalResponse;
use crate::protocols::ProtocolError;
use crate::read_subject::{declares_athlete_argument, requires_athlete_argument};
use crate::registry::chat_withholds;
use crate::security::{Audience, RuntimeTool};
use crate::tool_access::{
    coach_role_refusal, is_human_coach, withheld_call, Caller, CallerRole, Withheld,
};

impl UniversalExecutor {
    /// What [`withheld_call`] answers for this call with `args`, rendered
    /// ([`render_withheld`]): `None` when the call may run.
    ///
    /// The role comes from one of two places:
    ///
    /// - a chat turn binds the role its catalogue was built for
    ///   ([`Self::with_turn_role`]), and both flags are read from it, so a
    ///   call is refused exactly as the turn's catalogue
    ///   ([`offered_schemas`](crate::chat_catalogue::offered_schemas)) left
    ///   the tool out: an API key's turn and the A2A text turn offer a human
    ///   coach the coach tools, as `/mcp` does (carnet#1511, plan site 5).
    ///   The catalogue offers no admin tool to anyone ([`chat_withholds`]),
    ///   so such a turn is refused one whatever its role (carnet#1511 design
    ///   call 3);
    /// - otherwise the admin flag is `is_admin`, the live role
    ///   [`build_tool_context`](super::subject::build_tool_context) read, and
    ///   the coach flag is read live only where it can change the answer: a
    ///   coach tool, for a caller who is not an admin, over an external
    ///   transport for an MCP client or an A2A agent (the entry points that
    ///   bind [`Self::with_caller_writing_reply`]) and on Dravr's own
    ///   surfaces, where a coach-only tool goes to a human coach alone too
    ///   (carnet#1511 I7); and a tool whose every call names an athlete, on
    ///   any transport and for any caller, admins included (carnet#1511).
    ///
    /// An executor bound to neither (a nested dispatch inside a tool body) is
    /// decided for a caller with no role as well, the admin tools' own gate
    /// aside, so a nested call never widens what its caller was offered: a
    /// tool that names an athlete is refused there for want of the coach
    /// role, which no tool body dispatches. On Dravr's own surfaces a coach
    /// tool is the one exception: the coach flag read live above decides it
    /// there, as it decides it for a chat turn.
    pub(super) async fn access_refusal(
        &self,
        tool: &dyn RuntimeTool,
        (tool_name, args): (&str, &Value),
        user_id: Uuid,
        is_admin: bool,
        ceiling: Option<&GrantCeiling>,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        if self.turn_role.is_some() && chat_withholds(tool) {
            return render_withheld(tool_name, Withheld::Role(Audience::Admin)).map(Some);
        }
        let transport = ai_scope::effective_transport(self.transport);
        let external = !transport.is_some_and(Transport::is_first_party);
        let coach = if let Some(role) = self.turn_role {
            role.coach
        } else {
            // A tool whose every call names an athlete is the coach's
            // wherever it runs, in the app too (carnet#1511).
            let names_an_athlete = requires_athlete_argument(tool);
            let coach_tool = tool.audience() == Audience::Coach
                && !is_admin
                && (self.caller_writes_reply || !external);
            (coach_tool || names_an_athlete)
                && is_human_coach(self.resources.repos(), user_id).await
        };
        let caller = Caller {
            role: CallerRole {
                admin: self.turn_role.map_or(is_admin, |role| role.admin),
                coach,
            },
            grant: &self.scopes,
            transport,
            support_session: self.support_session.is_some(),
            ceiling,
        };
        let registry = self.resources.tool_registry();
        let answer = withheld_call(registry, tool_name, tool, &caller, args).or_else(|| {
            if self.caller_writes_reply || self.turn_role.is_some() {
                return None;
            }
            let role_less = Caller {
                role: CallerRole::default(),
                ..caller
            };
            let live_coach_rule = !external && tool.audience() == Audience::Coach;
            withheld_call(registry, tool_name, tool, &role_less, args).filter(|why| {
                *why != Withheld::Role(Audience::Admin)
                    && !(live_coach_rule && *why == Withheld::Role(Audience::Coach))
            })
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
                && self.client_can_step_up(user_id, ceiling).await
            {
                return Ok(Some(grant_ceiling_refusal::step_up(tool_name, &streams)));
            }
        }
        answer
            .map(|answer| render_withheld(tool_name, answer))
            .transpose()
    }

    /// Whether the calling client may be told to step up, by the rule
    /// `/mcp`'s 403 challenge follows ([`host_profile::may_step_up`]), for a
    /// call of `user_id` under `ceiling` and the Host Profile claim its own
    /// request declares. A call with no OAuth client (an API key, a
    /// session) may not.
    async fn client_can_step_up(&self, user_id: Uuid, ceiling: &GrantCeiling) -> bool {
        let Some(client_id) = self
            .credential
            .as_ref()
            .and_then(|credential| credential.client_id.as_deref())
        else {
            return false;
        };
        let repos = self.resources.repos();
        let call = CurableCall {
            client_id,
            user_id: &user_id.to_string(),
            client_grant_id: &ceiling.client_grant_id,
            request_tier: ceiling.request_host_tier,
        };
        host_profile::may_step_up(
            repos.oauth2_server.as_ref(),
            repos.host_profile_claims.as_ref(),
            call,
        )
        .await
    }
}

impl UniversalExecutor {
    /// Every refusal a call meets before anything it reads is looked up, in
    /// order, the first that applies: the scope, transport, role and tier gate
    /// under `ceiling`, the call's grant ([`Self::access_refusal`]); a write naming an athlete over an
    /// external transport; the coach's seat; a write after a member read
    /// (carnet#1422, carnet#1511 call 16); and last, an /mcp or A2A
    /// free-text write by a grant that read another member (call 30).
    pub(super) async fn call_refusal(
        &self,
        tool: &dyn RuntimeTool,
        tool_name: &str,
        args: &Value,
        user_uuid: Uuid,
        is_admin: bool,
        ceiling: Option<&GrantCeiling>,
    ) -> Result<Option<UniversalResponse>, ProtocolError> {
        let refusal = self
            .access_refusal(tool, (tool_name, args), user_uuid, is_admin, ceiling)
            .await?
            .or_else(|| self.external_athlete_write_refusal(tool, tool_name, args))
            .or_else(|| self.seat.refusal(self.resources.tool_registry(), tool_name))
            .or_else(|| {
                coach_seat::after_member_read_refusal(
                    self.resources.tool_registry(),
                    tool_name,
                    args,
                    // Off the subject's own seat the seat decides the
                    // caller's writes: a coach's room keeps only the group
                    // tools and the agent catalogue, whose writes store no
                    // free text (an agent installed from the store is a
                    // structured row, carnet#742), and a coach's thread
                    // withholds every write on the caller's account while
                    // its plan tools act on its athlete (carnet#1070), whose
                    // free text `plan_scope` decides by the athlete named
                    // (carnet#1511 call 16).
                    if self.seat.sender_is_subject() {
                        self.member_reads(user_uuid)
                    } else {
                        coach_seat::MemberReads::default()
                    },
                )
            });
        match refusal {
            Some(refusal) => Ok(Some(refusal)),
            None => self.grant_taint_refusal(tool_name, user_uuid).await,
        }
    }
}

impl UniversalExecutor {
    /// The refusal of a call to `tool`, which writes, that names an athlete
    /// in `args` over an external transport: the listing's rule
    /// (`tool_access::retain_offered`) at the dispatch chokepoint, for every
    /// role, so a write is accepted with a name exactly where it is offered
    /// one (carnet#1533). What names an athlete is what every write body
    /// reads as one ([`coach_seat::names_an_athlete`]): a blank or null
    /// `athlete` names no one.
    pub(super) fn external_athlete_write_refusal(
        &self,
        tool: &dyn RuntimeTool,
        tool_name: &str,
        args: &Value,
    ) -> Option<UniversalResponse> {
        // External as the tool bodies read it (`ai_scope::serving_external`):
        // a named transport that is not first-party. An executor serving no
        // transport is in-process, never external.
        let external = ai_scope::effective_transport(self.transport)
            .is_some_and(|transport| !transport.is_first_party());
        let writes = tool.capabilities().contains(ToolCapabilities::WRITES_DATA);
        (external
            && writes
            && declares_athlete_argument(tool)
            && coach_seat::names_an_athlete(args))
        .then(|| first_party_tools::external_write_refusal(tool_name))
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
        Withheld::NeverExternal => Ok(super::tier_gate::never_external(tool_name)),
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
