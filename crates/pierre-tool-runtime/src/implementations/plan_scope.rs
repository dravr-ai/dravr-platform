// ABOUTME: Whose training plan get_training_plan / save_training_plan act on — the caller's own, or a coached athlete's
// ABOUTME: The athlete= path needs the coach attachment, the athlete's consent, one home tenant and a direct chat; every refusal is an honest error
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan scope
//!
//! The plan a tool reads or writes is an athlete's one season plan, under the
//! athlete's own tenant — the plan their DM, `/plan` and home screen read,
//! whichever agent laid it. By default that athlete is the caller. A group's
//! human coach — the Dravr user attached through a coach invite,
//! `coaching_groups.coach_user_id` — can name a roster athlete with
//! `athlete=` and read or write *that* athlete's season instead.
//!
//! Who writes is separate from whose plan it is. [`PlanWriter`] is the
//! requester — their tenant and user, where their agent is visible — and the
//! agent their turn answers as. What a coach saves for an athlete records the
//! coach's conversation agent as its author, and is validated against that
//! agent's package, resolved as the coach.
//!
//! Four gates, all required, each answered with the tool's own error text so
//! the model can say plainly why it could not act:
//!
//! 1. **A direct chat.** A room turn's reply is posted to the whole room, so
//!    acting on an athlete's plan there would publish it. `athlete=` is
//!    allowed only when the call carries no conversation (MCP-direct) or the
//!    conversation resolves under the requester's own tenant with no group
//!    bound; a conversation id that does not resolve there IS a shared room
//!    (it lives under the bot tenant).
//! 2. **The agent attachment.** The athlete must be an active member of a
//!    group the requester is `coach_user_id` of. Owner and admin roles do not
//!    qualify — on a channel-bound group the owner is whoever spoke first.
//! 3. **The athlete's consent.** The gate `get_group_member_activities`
//!    applies to a group's coach: the member's own `coach_sharing_consent`.
//!    Joining grants it (ADR-002: membership is consent to share with the
//!    coach) and `/group consent coach no` revokes it, so nothing here ships
//!    with zero athlete-side control. The group's `peer_data_sharing` switch
//!    governs what members see of each other, not their coach.
//! 4. **One home tenant.** The plan lives under the athlete's own tenant. An
//!    athlete who belongs to several would read an agent-written plan on one
//!    surface and not another, so that case is refused rather than guessed.
//!
//! Gates 2 to 4 are [`coached_athlete_access`], the by-id check the route
//! that opens a coach's athlete thread runs too: a name is resolved to one
//! member here, and the check decides by their id.
//!
//! ## A thread bound to one athlete
//!
//! A coach's thread opened from a group's roster names its athlete
//! ([`ConversationRecord::subject`], carnet#1070). There every call acts on
//! that athlete's plan: with no `athlete`, or one their name contains, the
//! subject is resolved by id against the thread's group; any other name is
//! refused, and so is the coach's own plan, which is never acted on there.
//! The gates run again on every call, so a consent revoked mid-thread refuses
//! the next one.
//!
//! A write into a tenant other than the requester's also honours that
//! tenant's tool-disable configuration, which the dispatch chokepoint checked
//! against the requester's tenant only.

use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{ConversationRecord, ConversationSubject, TenantId};
use pierre_runtime_context::DataContext;
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use pierre_tools_core::ToolResult;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use super::training_plan_authorship::resolve_turn_agent;
use crate::context::ToolExecutionContext;
use crate::guardian::{
    tenant_disabled_message, tenant_tool_enabled, TENANT_DISABLED_ERROR_CODE,
    TENANT_DISABLED_REASON,
};

/// Whose plan a training-plan tool reads or writes, and who is writing.
#[derive(Debug)]
pub struct PlanScope {
    /// Tenant the plan lives under.
    pub tenant: TenantId,
    /// Athlete the plan belongs to.
    pub user_id: Uuid,
    /// The requester and the agent their turn answers as.
    pub writer: PlanWriter,
    /// Roster display name of the athlete when the caller acts for someone
    /// else; `None` in self scope.
    pub acting_for: Option<String>,
}

/// Who writes to the plan: the requester and the agent their turn answers as.
///
/// The tenant and user are the requester's own, not the athlete's, because
/// they are where the writer's agent is visible: its package, its title, and
/// whether an author named on the plan is this writer.
#[derive(Debug)]
pub struct PlanWriter {
    /// The agent the requester's turn answers as; `None` for a direct call
    /// that names none, or a conversation bound to no agent.
    pub agent_id: Option<String>,
    /// The requester's own tenant.
    pub tenant: TenantId,
    /// The requester.
    pub user_id: Uuid,
}

/// What a plan tool knows about the call before it resolves whose plan it is.
#[derive(Debug)]
pub struct PlanScopeRequest<'a> {
    /// The executing call: requester, runtime and originating conversation.
    pub context: &'a ToolExecutionContext,
    /// The requester's own tenant, from the call.
    pub requester_tenant: TenantId,
    /// The originating conversation, when it resolved under the requester's
    /// own tenant.
    pub conversation: Option<&'a ConversationRecord>,
    /// The `agent_id` argument, used only for a call with no conversation and
    /// no turn agent.
    pub arg_agent: Option<String>,
    /// The `athlete` argument: a roster display name, or `None` for self scope.
    pub athlete: Option<&'a str>,
    /// The tool asking, for the cross-tenant tool-disable check and the log line.
    pub tool_name: &'a str,
}

/// The one coached member a name query matched, with the group row that
/// authorizes acting for them.
pub(crate) struct CoachedMatch {
    /// A group the requester coaches the member in: one where the member
    /// shares with them when any does, so a refusal names the most permissive
    /// row; else the first.
    pub(crate) group: CoachingGroup,
    /// The member.
    pub(crate) user_id: Uuid,
    /// Their display name ([`fetch_user_display_name`]).
    pub(crate) display_name: String,
    /// Whether the member shares with the requester in `group`.
    pub(crate) consented: bool,
}

/// Why a name query matched no one coached member.
pub(crate) enum MemberMiss {
    /// The query matched only the requester.
    Requester,
    /// No live member of a group the requester coaches matched.
    NoMatch,
    /// Several distinct members matched: their display names, sorted.
    Ambiguous(Vec<String>),
}

impl MemberMiss {
    /// What the model relays for a query `query` that resolved to no one
    /// member; `yourself` says what to do when it named the requester.
    pub(crate) fn message(&self, query: &str, yourself: &str) -> String {
        match self {
            Self::Requester => format!("'{query}' is you — {yourself}"),
            Self::NoMatch => format!(
                "No athlete matching '{query}' in a group you coach — the athlete must be a \
                 member of a group you are attached to as its coach (a coach invite), not merely \
                 a group you belong to."
            ),
            Self::Ambiguous(names) => format!(
                "'{query}' is ambiguous — several athletes you coach match: {}. Use a more \
                 specific name.",
                names.join(", ")
            ),
        }
    }
}

/// The one live member of a group `requester` is the human coach of whose
/// display name contains `query`, case aside.
///
/// Walked from the groups the requester is attached to as their coach, never
/// from the groups they merely belong to. The same athlete in two coached
/// groups is one person; the requester is never their own athlete.
///
/// The outer `Err` is a repository failure; the inner `Err` says why no one
/// member matched.
///
/// # Errors
///
/// Returns the repository error from the group or member lookups.
pub(crate) async fn match_coached_member(
    data: &DataContext,
    requester: Uuid,
    query: &str,
) -> AppResult<Result<CoachedMatch, MemberMiss>> {
    let repos = data.repos();
    let query_lower = query.to_lowercase();
    let mut matches: Vec<CoachedMatch> = Vec::new();
    let mut requester_matched = false;
    for group in repos.groups.list_groups_coached_by(requester).await? {
        if !group.is_active {
            continue;
        }
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if member.left_at.is_some() {
                continue;
            }
            let display_name = fetch_user_display_name(data.repos(), member.user_id).await;
            if !display_name.to_lowercase().contains(&query_lower) {
                continue;
            }
            if member.user_id == requester {
                requester_matched = true;
                continue;
            }
            matches.push(CoachedMatch {
                group: group.clone(),
                user_id: member.user_id,
                display_name,
                consented: member.coach_sharing_consent,
            });
        }
    }

    let Some(first) = matches.first() else {
        return Ok(Err(if requester_matched {
            MemberMiss::Requester
        } else {
            MemberMiss::NoMatch
        }));
    };
    // Ambiguous only when DISTINCT athletes match — the same athlete in two
    // coached groups is one person.
    if matches.iter().any(|m| m.user_id != first.user_id) {
        let mut names: Vec<String> = matches.iter().map(|m| m.display_name.clone()).collect();
        names.sort_unstable();
        names.dedup();
        return Ok(Err(MemberMiss::Ambiguous(names)));
    }
    let at = matches.iter().position(|m| m.consented).unwrap_or(0);
    Ok(Ok(matches.swap_remove(at)))
}

/// An honest refusal in the tool's own error shape.
fn refusal(message: &str) -> ToolResult {
    ToolResult::error(json!({ "error": message }))
}

/// Resolve whose plan the call acts on.
///
/// The outer `Err` is a repository failure; the inner `Err` is a refusal the
/// tool returns as its result, worded for the model to relay.
///
/// # Errors
///
/// Returns the repository error from any lookup the resolution needs.
pub async fn resolve_plan_scope(
    mut request: PlanScopeRequest<'_>,
) -> AppResult<Result<PlanScope, ToolResult>> {
    let writer = PlanWriter {
        agent_id: resolve_turn_agent(
            request.context,
            request.conversation,
            request.arg_agent.take(),
        ),
        tenant: request.requester_tenant,
        user_id: request.context.user_id,
    };
    let query = request.athlete.map(str::trim).filter(|q| !q.is_empty());

    // A thread bound to one athlete works on that athlete's plan whether or
    // not the call names them, and never on the coach's own.
    if let Some(subject) = request.conversation.and_then(ConversationRecord::subject) {
        return resolve_subject_scope(&request, writer, subject, query).await;
    }

    let Some(query) = query else {
        return Ok(Ok(PlanScope {
            tenant: request.requester_tenant,
            user_id: request.context.user_id,
            writer,
            acting_for: None,
        }));
    };

    // Gate 1: a room would publish the athlete's plan. A conversation id the
    // requester's own tenant cannot resolve is a shared room's row under the
    // bot tenant, so it is refused exactly like a group-bound conversation.
    if request.context.conversation_id.is_some()
        && request
            .conversation
            .is_none_or(|conv| conv.group_id.is_some())
    {
        return Ok(Err(refusal(
            "act on an athlete's plan from your direct chat with me — a room would publish it \
             to every member. The athlete can post their own plan in the room with `/plan share`.",
        )));
    }

    let requester = request.context.user_id;
    let data = request.context.resources.data();

    // The name, resolved to one live member of a group the requester is
    // attached to as its coach — never one they merely belong to.
    let resolved = match match_coached_member(&data, requester, query).await? {
        Ok(resolved) => resolved,
        Err(miss) => {
            return Ok(Err(refusal(
                &miss.message(query, "omit `athlete` to act on your own plan."),
            )));
        }
    };

    // Gates 2 to 4, by id. The match prefers a group where the athlete shares
    // with the coach, so a consent refusal, when there is one, names the most
    // permissive row. Only a membership that changed since the match fails the
    // first two, and that reads as no match.
    match coached_athlete_access(data.repos(), requester, resolved.user_id, resolved.group.id)
        .await?
    {
        Ok(access) => acting_for(&request, writer, access).await,
        Err(refused) => Ok(Err(gate_refusal(&refused, || {
            MemberMiss::NoMatch.message(query, "omit `athlete` to act on your own plan.")
        }))),
    }
}

/// [`resolve_plan_scope`] in a thread bound to `subject`: the subject's plan,
/// by id against the thread's group, re-checked on every call.
///
/// `query` is the call's `athlete` argument. A name the subject's own roster
/// name contains is the subject — in the thread about them, a shared first
/// name means them. Any other name is refused: another athlete has a thread
/// of their own, and the coach's own plan is never acted on here.
async fn resolve_subject_scope(
    request: &PlanScopeRequest<'_>,
    writer: PlanWriter,
    subject: ConversationSubject,
    query: Option<&str>,
) -> AppResult<Result<PlanScope, ToolResult>> {
    let requester = request.context.user_id;
    let data = request.context.resources.data();
    let access = match coached_athlete_access(
        data.repos(),
        requester,
        subject.athlete_user_id,
        subject.group_id,
    )
    .await?
    {
        Ok(access) => access,
        Err(refused) => {
            return Ok(Err(gate_refusal(&refused, || {
                match refused {
                    CoachedAthleteRefusal::NotGroupCoach => {
                        "This thread works on an athlete of a group you no longer coach, so \
                         their plan is out of reach here."
                    }
                    _ => {
                        "This thread's athlete is no longer a member of the group you coach \
                         them in, so their plan is out of reach here."
                    }
                }
                .to_owned()
            })));
        }
    };

    if let Some(query) = query {
        if !access
            .display_name
            .to_lowercase()
            .contains(&query.to_lowercase())
        {
            let name = &access.display_name;
            let own = match match_coached_member(&data, requester, query).await? {
                Ok(other) => {
                    return Ok(Err(refusal(&format!(
                        "This thread works on {name}'s plan — open {}'s own thread from Group \
                         info to work on theirs.",
                        other.display_name
                    ))));
                }
                Err(MemberMiss::Requester) => true,
                // A coach attached to a group holds no membership in it, so
                // their own name is asked of their own row.
                Err(MemberMiss::NoMatch | MemberMiss::Ambiguous(_)) => {
                    fetch_user_display_name(data.repos(), requester)
                        .await
                        .to_lowercase()
                        .contains(&query.to_lowercase())
                }
            };
            let message = if own {
                format!(
                    "This thread works on {name}'s plan — act on your own plan from your own \
                     conversation with me."
                )
            } else {
                format!(
                    "This thread works on {name}'s plan, and '{query}' does not name them — \
                     omit `athlete` to act on {name}'s plan."
                )
            };
            return Ok(Err(refusal(&message)));
        }
    }

    acting_for(request, writer, access).await
}

/// What the model relays for a gate refusal. `unreached` words the two that
/// fail before the athlete is shown to be the requester's — those name nobody.
fn gate_refusal(refused: &CoachedAthleteRefusal, unreached: impl FnOnce() -> String) -> ToolResult {
    let message = match refused {
        CoachedAthleteRefusal::NotGroupCoach | CoachedAthleteRefusal::NotAMember => unreached(),
        CoachedAthleteRefusal::CoachSharingRevoked { athlete, group } => format!(
            "{athlete} has stopped sharing their data with you, the coach of '{group}'. They can \
             share it again with `/group consent coach yes`."
        ),
        CoachedAthleteRefusal::NoTenant { athlete } => {
            format!("{athlete} belongs to no tenant, so there is nowhere to keep their plan.")
        }
        CoachedAthleteRefusal::SeveralTenants { athlete } => format!(
            "{athlete} belongs to several tenants — acting on their plan is not supported, \
             because the plan would be visible on one of their surfaces and missing on another."
        ),
    };
    refusal(&message)
}

/// The scope of a call the gates let act for `access`'s athlete, once the
/// athlete's tenant allows the tool.
async fn acting_for(
    request: &PlanScopeRequest<'_>,
    writer: PlanWriter,
    access: CoachedAthleteAccess,
) -> AppResult<Result<PlanScope, ToolResult>> {
    // The dispatch chokepoint checked the tool against the requester's tenant;
    // a plan filed under another tenant honours THAT tenant's configuration.
    if access.tenant != request.requester_tenant
        && !tenant_tool_enabled(&request.context.resources, access.tenant, request.tool_name).await
    {
        return Ok(Err(ToolResult::error(json!({
            "error": tenant_disabled_message(request.tool_name),
            "error_code": TENANT_DISABLED_ERROR_CODE,
            "reason": TENANT_DISABLED_REASON,
        }))));
    }

    info!(
        requester = %request.context.user_id,
        athlete = %access.athlete_user_id,
        group_id = %access.group.id,
        tool = %request.tool_name,
        writer_agent = writer.agent_id.as_deref().unwrap_or("none"),
        subject_thread = request.conversation.and_then(ConversationRecord::subject).is_some(),
        "coach acting on a coached athlete's training plan"
    );
    Ok(Ok(PlanScope {
        tenant: access.tenant,
        user_id: access.athlete_user_id,
        writer,
        acting_for: Some(access.display_name),
    }))
}
