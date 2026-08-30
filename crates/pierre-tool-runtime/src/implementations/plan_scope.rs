// ABOUTME: Whose training plan get_training_plan / save_training_plan act on — the caller's own, or a coached athlete's
// ABOUTME: The athlete= path needs the coach attachment, the athlete's consent, one home tenant and a direct chat; every refusal is an honest error
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Plan scope
//!
//! Both plan tools used to act on the caller alone. A group's human coach —
//! the Dravr user attached through a coach invite, `coaching_groups.
//! coach_user_id` — can now name a roster athlete with `athlete=` and read or
//! write *that* athlete's plan, under the athlete's own tenant and selected
//! coach, which is where the athlete's DM and `/plan` read it.
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
//! 2. **The coach attachment.** The athlete must be an active member of a
//!    group the requester is `coach_user_id` of. Owner and admin roles do not
//!    qualify — on a channel-bound group the owner is whoever spoke first.
//! 3. **The athlete's consent.** The same two gates
//!    `get_group_member_activities` applies: the group's `peer_data_sharing`
//!    switch and the member's own `peer_sharing_consent`. Consent is the
//!    athlete's grant; nothing here ships with zero athlete-side control.
//! 4. **One home tenant.** The plan lives under the athlete's own tenant. An
//!    athlete who belongs to several would read a coach-written plan on one
//!    surface and not another, so that case is refused rather than guessed.
//!
//! A write into a tenant other than the requester's also honours that
//! tenant's tool-disable configuration, which the dispatch chokepoint checked
//! against the requester's tenant only.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_tools_core::ToolResult;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use super::training_plans::resolve_coach_slug;
use crate::athlete_display_name::fetch_user_display_name;
use crate::context::ToolExecutionContext;
use crate::guardian::{
    tenant_disabled_message, tenant_tool_enabled, TENANT_DISABLED_ERROR_CODE,
    TENANT_DISABLED_REASON,
};

/// Whose plan a training-plan tool reads or writes.
pub struct PlanScope {
    /// Tenant the plan lives under.
    pub tenant: TenantId,
    /// Athlete the plan belongs to.
    pub user_id: Uuid,
    /// Coach persona slug the plan is filed under.
    pub coach_slug: Option<String>,
    /// Roster display name of the athlete when the caller acts for someone
    /// else; `None` in self scope.
    pub acting_for: Option<String>,
}

/// What a plan tool knows about the call before it resolves whose plan it is.
pub struct PlanScopeRequest<'a> {
    /// The executing call: requester, runtime and originating conversation.
    pub context: &'a ToolExecutionContext,
    /// The requester's own tenant, from the call.
    pub requester_tenant: TenantId,
    /// The originating conversation, when it resolved under the requester's
    /// own tenant.
    pub conversation: Option<&'a ConversationRecord>,
    /// The `coach_id` argument, used only for a conversation-less call.
    pub arg_coach: Option<String>,
    /// The `athlete` argument: a roster display name, or `None` for self scope.
    pub athlete: Option<&'a str>,
    /// The tool asking, for the cross-tenant tool-disable check and the log line.
    pub tool_name: &'a str,
}

/// A coached athlete whose display name matched the query, with the group
/// row that authorizes acting for them.
struct AthleteMatch {
    group_id: Uuid,
    group_name: String,
    group_allows_sharing: bool,
    display_name: String,
    user_id: Uuid,
    consented: bool,
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
    request: PlanScopeRequest<'_>,
) -> AppResult<Result<PlanScope, ToolResult>> {
    let Some(query) = request.athlete.map(str::trim).filter(|q| !q.is_empty()) else {
        return Ok(Ok(PlanScope {
            tenant: request.requester_tenant,
            user_id: request.context.user_id,
            coach_slug: resolve_coach_slug(request.conversation, request.arg_coach),
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
    let repos = data.repos();
    let query_lower = query.to_lowercase();

    // Gate 2: the coach attachment, walked from the groups the requester is
    // attached to as coach — never from the groups they merely belong to.
    let mut matches: Vec<AthleteMatch> = Vec::new();
    let mut requester_matched = false;
    for group in repos.groups.list_groups_coached_by(requester).await? {
        if !group.is_active {
            continue;
        }
        for member in repos.groups.list_members(&group.id.to_string()).await? {
            if member.left_at.is_some() {
                continue;
            }
            let display_name = fetch_user_display_name(&data, member.user_id).await;
            if !display_name.to_lowercase().contains(&query_lower) {
                continue;
            }
            if member.user_id == requester {
                requester_matched = true;
                continue;
            }
            matches.push(AthleteMatch {
                group_id: group.id,
                group_name: group.name.clone(),
                group_allows_sharing: group.peer_data_sharing,
                display_name,
                user_id: member.user_id,
                consented: member.peer_sharing_consent,
            });
        }
    }

    let Some(first) = matches.first() else {
        if requester_matched {
            return Ok(Err(refusal(&format!(
                "'{query}' is you — omit `athlete` to act on your own plan."
            ))));
        }
        return Ok(Err(refusal(&format!(
            "No athlete matching '{query}' in a group you coach — the athlete must be a member \
             of a group you are attached to as its coach (a coach invite), not merely a group \
             you belong to."
        ))));
    };

    // Ambiguous only when DISTINCT athletes match — the same athlete in two
    // coached groups is one person.
    if matches.iter().any(|m| m.user_id != first.user_id) {
        let mut names: Vec<&str> = matches.iter().map(|m| m.display_name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        return Ok(Err(refusal(&format!(
            "'{query}' is ambiguous — several athletes you coach match: {}. Use a more specific name.",
            names.join(", ")
        ))));
    }

    // Gate 3: the athlete's own grant. Prefer a group where both gates are
    // open so the refusal, when there is one, names the most permissive row.
    let resolved = matches
        .iter()
        .find(|m| m.group_allows_sharing && m.consented)
        .unwrap_or(first);
    if !resolved.group_allows_sharing {
        return Ok(Err(refusal(&format!(
            "Peer data sharing is disabled for group '{}'.",
            resolved.group_name
        ))));
    }
    if !resolved.consented {
        return Ok(Err(refusal(&format!(
            "{} hasn't shared their data with the group yet. They can opt in with `/group consent yes`.",
            resolved.display_name
        ))));
    }

    // Gate 4: the athlete's one home tenant — where their DM saves and reads.
    let athlete = resolved.user_id;
    let tenants = repos.tenants.list_for_user(athlete).await?;
    let athlete_tenant = match tenants.as_slice() {
        [tenant] => tenant.id,
        [] => {
            return Ok(Err(refusal(&format!(
                "{} belongs to no tenant, so there is nowhere to keep their plan.",
                resolved.display_name
            ))));
        }
        _ => {
            return Ok(Err(refusal(&format!(
                "{} belongs to several tenants — acting on their plan is not supported, because \
                 the plan would be visible on one of their surfaces and missing on another.",
                resolved.display_name
            ))));
        }
    };

    // The dispatch chokepoint checked the tool against the requester's tenant;
    // a plan filed under another tenant honours THAT tenant's configuration.
    if athlete_tenant != request.requester_tenant
        && !tenant_tool_enabled(
            &request.context.resources,
            athlete_tenant,
            request.tool_name,
        )
        .await
    {
        return Ok(Err(ToolResult::error(json!({
            "error": tenant_disabled_message(request.tool_name),
            "error_code": TENANT_DISABLED_ERROR_CODE,
            "reason": TENANT_DISABLED_REASON,
        }))));
    }

    let coach_slug = repos
        .tenants
        .get_selected_coach(athlete_tenant, athlete)
        .await?;
    info!(
        requester = %requester,
        athlete = %athlete,
        group_id = %resolved.group_id,
        tool = %request.tool_name,
        "coach acting on a coached athlete's training plan"
    );
    Ok(Ok(PlanScope {
        tenant: athlete_tenant,
        user_id: athlete,
        coach_slug,
        acting_for: Some(resolved.display_name.clone()),
    }))
}
