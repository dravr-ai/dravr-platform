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
//!    allowed only when the call carries no conversation (MCP-direct, which
//!    reads but never writes — see below) or the conversation resolves under
//!    the requester's own tenant with no group bound, on a turn marked a
//!    direct message; a conversation id that does not resolve there IS a
//!    shared room (it lives under the bot tenant), and a messaging group chat
//!    whose binding failed keeps no group either (carnet#1422). The room
//!    classifier (`call_room::CallRoom::of`) decides.
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
//! member ([`match_coached_member`], shared with the read scope in
//! [`crate::read_subject`]), and the check decides by their id.
//!
//! What a tool then reads of the athlete runs under the athlete's own
//! provider AI consents, never the coach's ([`PlanScope::read_as_athlete`],
//! carnet#1419). Their wellness — sleep, HRV, resting heart rate — takes the
//! athlete's `coach_wellness_read` grant on top of the gates
//! ([`PlanScope::wellness_withheld`], carnet#1423).
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
//!
//! ## The athlete's record
//!
//! Once every gate let a call act for a coached athlete, the call is recorded
//! on the athlete's own record, naming the tool, before it reads or writes
//! anything of theirs (carnet#1445): a training read, since every plan tool
//! reads the plan and the training it is laid against.
//! `get_training_plan` asked for the plan's state also reads their recovery,
//! and records a wellness read too ([`PlanScope::record_wellness_read`]). A
//! refused call records nothing.
//!
//! ## Writes stay on Dravr's own surfaces
//!
//! A coach writes a coached athlete's plan — a draft save, a publish, a
//! discard — from Dravr's app or channels only. The athlete's consent was
//! given inside Dravr; an MCP, A2A or API-key client acting for the coach is
//! outside it, so a tool that declares a write ([`ToolRegistry::may_change_data`])
//! naming an athlete, or run in a thread bound to one, is refused over an
//! external transport before anything about the athlete is read
//! (`external_write_refusal`, the calendar's refusal too, carnet#1533).
//! Reads through `athlete=` are unchanged, gate 1 included: a direct MCP call
//! carries no conversation and reads as a direct chat.
//!
//! [`ToolRegistry::may_change_data`]: crate::registry::ToolRegistry::may_change_data
//!
//! ## A coach reading a published plan
//!
//! Outside the plan tools a coach reads an athlete's live plan read-only —
//! Group info's view, `/plan <athlete>` answered to them alone, and the plan
//! status the coach's room may see (carnet#1244). None of those is a tool
//! call the chokepoint checked, so [`coached_plan_read_access`] runs gates 2
//! to 4 and the athlete's tenant allowing `get_training_plan`, then records
//! the read on the athlete's record under the surface's name. Gate 1 is each
//! surface's own: the view and the private reply reach the coach alone, and
//! the room's status carries no week of the plan.

use std::future::Future;
use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{ConversationRecord, ConversationSubject, TenantId};
use pierre_database::repositories::CoachReadClass;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_services::coached_athlete::{
    coached_athlete_access, CoachedAthleteAccess, CoachedAthleteRefusal,
};
use pierre_services::provider_notice;
use pierre_tools_core::ToolResult;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use super::call_room::CallRoom;
use super::training_plan_authorship::resolve_turn_agent;
use crate::context::ToolExecutionContext;
use crate::first_party_tools::EXTERNAL_TRANSPORT_REASON;
use crate::guardian::{
    tenant_disabled_message, tenant_tool_enabled, TENANT_DISABLED_ERROR_CODE,
    TENANT_DISABLED_REASON,
};
use crate::read_subject::{
    gate_message, match_coached_member, names_subject, other_name, record_coach_read,
    thread_unreached, GateObject, MemberMiss, OtherName, WellnessWithheld,
};
use crate::runtime::ToolRuntime;

/// Whose plan a training-plan tool reads or writes, and who is writing.
#[derive(Debug)]
pub struct PlanScope {
    /// Tenant the plan lives under.
    pub tenant: TenantId,
    /// Athlete the plan belongs to.
    pub user_id: Uuid,
    /// The requester and the agent their turn answers as.
    pub writer: PlanWriter,
    /// Whether the plan is the requester's own or a coached athlete's.
    pub(crate) subject: PlanSubject,
}

/// Whose plan a [`PlanScope`] acts on.
///
/// One enum rather than three options that are always set or empty
/// together: a coached athlete's scope carries their name, the group and
/// their consents, and the caller's own carries none of them.
#[derive(Debug)]
pub(crate) enum PlanSubject {
    /// The requester's own plan.
    Own,
    /// A coached athlete's plan, which the requester acts on as their coach.
    Coached {
        /// What the coaching gates answered for this call: the athlete's
        /// roster name, the group whose gates let the requester act for them
        /// — the room a publish of their plan is said in (carnet#1249) — and
        /// what they let the requester do beyond reading their training
        /// ([`CoachedAthleteAccess::consents`]). Boxed: the own scope
        /// carries nothing.
        access: Box<CoachedAthleteAccess>,
    },
}

impl PlanScope {
    /// The athlete's roster display name when the caller acts for a coached
    /// athlete; `None` in self scope.
    #[must_use]
    pub(crate) fn acting_for(&self) -> Option<&str> {
        match &self.subject {
            PlanSubject::Own => None,
            PlanSubject::Coached { access } => Some(&access.display_name),
        }
    }

    /// Why this call leaves the athlete's wellness — their sleep, HRV and
    /// resting heart rate — out, or `None` when it reads it.
    ///
    /// The athlete reads their own. A human coach acting for them reads it
    /// only under the athlete's `coach_wellness_read` grant, never under
    /// their coach sharing alone (carnet#1423), and is told so in the one
    /// wording every coach's read gives ([`WellnessWithheld::note`]).
    #[must_use]
    pub(crate) fn wellness_withheld(&self) -> Option<WellnessWithheld> {
        match &self.subject {
            PlanSubject::Own => None,
            PlanSubject::Coached { access } => {
                (!access.consents.wellness_read).then(|| WellnessWithheld {
                    athlete: access.display_name.clone(),
                })
            }
        }
    }

    /// Record, on the coached athlete's own record, that `tool` reads their
    /// wellness for the requester (carnet#1445): only a coach's read under
    /// the athlete's wellness grant does — the athlete's own read records
    /// nothing, and without the grant nothing of it is read
    /// ([`Self::wellness_withheld`]).
    ///
    /// Called once the read is decided and before anything of their wellness
    /// is read. The training read of the same call was recorded when the
    /// scope resolved.
    ///
    /// # Errors
    ///
    /// Returns the repository error from the insert: the read is refused
    /// rather than left unrecorded.
    pub(crate) async fn record_wellness_read(
        &self,
        repos: &RepositoryRegistry,
        tool: &str,
    ) -> AppResult<()> {
        match &self.subject {
            PlanSubject::Coached { access } if access.consents.wellness_read => {
                record_coach_read(
                    repos,
                    access,
                    self.writer.user_id,
                    tool,
                    CoachReadClass::Wellness,
                )
                .await
            }
            PlanSubject::Coached { .. } | PlanSubject::Own => Ok(()),
        }
    }

    /// Run `read`, a read of this scope's athlete's data, under the athlete's
    /// own AI consents.
    ///
    /// A coach acting for a coached athlete reads the athlete's providers as
    /// the athlete allowed AI to read them, never as the coach did, and the
    /// call's result is filtered the same way (carnet#1419). In self scope the
    /// athlete is the caller, whose consents the dispatch chokepoint already
    /// declared.
    pub(crate) async fn read_as_athlete<F: Future>(
        &self,
        repos: &RepositoryRegistry,
        read: F,
    ) -> F::Output {
        if matches!(self.subject, PlanSubject::Own) {
            return read.await;
        }
        provider_notice::under_subject_ai_consent(repos, self.tenant.as_uuid(), self.user_id, read)
            .await
    }
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

/// The tool whose tenant configuration governs a coach's read of an
/// athlete's plan outside a tool call.
const PLAN_READ_TOOL: &str = "get_training_plan";

/// Why a group's human coach may not read a coached athlete's live plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoachedPlanRefusal {
    /// A coaching gate refused: the coach attachment, the membership, the
    /// athlete's consent, or their one home tenant.
    Coaching(CoachedAthleteRefusal),
    /// The athlete's tenant has `get_training_plan` turned off.
    PlansDisabled {
        /// The athlete's roster name.
        athlete: String,
    },
}

/// Whether `coach` may read `athlete`'s live plan as the human coach of
/// `group_id`: the coaching gates ([`coached_athlete_access`]), then the
/// athlete's tenant allowing `get_training_plan`.
///
/// Once both let the read through, it is recorded on the athlete's own
/// record under `source` — the tool that reads, or the surface's
/// [`CoachReadSurface`](pierre_database::repositories::CoachReadSurface)
/// label — before anything of the plan is read
/// (carnet#1445). Every caller reads the plan on an `Ok`.
///
/// The outer `Err` is a repository failure, the record's included: the read
/// is refused rather than left unrecorded. The inner `Err` is the refusal,
/// which each surface words its own way, and records nothing.
///
/// # Errors
///
/// Returns the repository error from the group, member or tenant lookups, or
/// from the record's insert.
pub async fn coached_plan_read_access(
    resources: &Arc<dyn ToolRuntime>,
    coach: Uuid,
    athlete: Uuid,
    group_id: Uuid,
    source: &str,
) -> AppResult<Result<CoachedAthleteAccess, CoachedPlanRefusal>> {
    let access = match coached_athlete_access(resources.repos(), coach, athlete, group_id).await? {
        Ok(access) => access,
        Err(refused) => return Ok(Err(CoachedPlanRefusal::Coaching(refused))),
    };
    if !tenant_tool_enabled(resources, access.tenant, PLAN_READ_TOOL).await {
        return Ok(Err(CoachedPlanRefusal::PlansDisabled {
            athlete: access.display_name,
        }));
    }
    record_coach_read(
        resources.repos(),
        &access,
        coach,
        source,
        CoachReadClass::Training,
    )
    .await?;
    // What the caller derives from here on carries the athlete's plan
    // (carnet#1174).
    ai_scope::note_member_read(access.athlete_user_id);
    Ok(Ok(access))
}

/// An honest refusal in the tool's own error shape.
fn refusal(message: &str) -> ToolResult {
    ToolResult::error(json!({ "error": message }))
}

/// The refusal of a coach's write to a coached athlete's `object` (their
/// "plan", their "calendar") made over an external transport, carrying the
/// `details.reason` a client branches on.
///
/// The one sentence for that refusal: the plan and calendar tools relay it
/// (`external_write_refusal`), and the plan card's REST publish and discard
/// return it, since they write the draft without going through a tool.
#[must_use]
pub fn external_write_error(object: &str) -> AppError {
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        format!(
            "A coach writes an athlete's {object} from Dravr's own app or channels, not over \
             this interface."
        ),
    );
    error.details = Some(Box::new(json!({ "reason": EXTERNAL_TRANSPORT_REASON })));
    error
}

/// The tool's refusal of a coach's write to a coached athlete's `object`
/// served over an external transport ([`external_write_error`]), or `None` on
/// Dravr's own surfaces.
///
/// The athlete consented to their coach inside Dravr. An MCP, A2A or API-key
/// client acting for the coach is outside that agreement, so it writes
/// nothing of the athlete's; what it reads through `athlete=` is not this
/// gate's (carnet#1533).
pub(super) fn external_write_refusal(object: &str) -> Option<ToolResult> {
    ai_scope::serving_external().then(|| {
        ToolResult::error(json!({
            "reason": EXTERNAL_TRANSPORT_REASON,
            "error": external_write_error(object).message,
        }))
    })
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

    // Another athlete's plan is written only from Dravr's own surfaces:
    // refused before anything about the athlete is read. A thread bound to
    // one athlete counts exactly as `athlete=` does. Whether the tool writes
    // is what it declares, so a plan tool added later is held here too.
    let for_athlete = query.is_some()
        || request
            .conversation
            .and_then(ConversationRecord::subject)
            .is_some();
    if for_athlete
        && request
            .context
            .resources
            .tool_registry()
            .may_change_data(request.tool_name)
    {
        if let Some(refused) = external_write_refusal("plan") {
            return Ok(Err(refused));
        }
    }

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
            subject: PlanSubject::Own,
        }));
    };

    // Gate 1: a room would publish the athlete's plan. Anything the room
    // classifier does not call private is refused: a group-bound
    // conversation, one the requester's own tenant cannot resolve (a shared
    // room's row under the bot tenant), and a chat with no group on a turn
    // not marked a direct message (carnet#1422).
    if !CallRoom::of(request.context, request.conversation).is_private() {
        return Ok(Err(refusal(
            "act on an athlete's plan from your direct chat with me — a room would publish it \
             to every member. The athlete can post their own plan in the room with `/plan share`.",
        )));
    }

    let requester = request.context.user_id;
    let data = request.context.resources.data();

    // The name, resolved to one live member of a group the requester is
    // attached to as its coach — never one they merely belong to.
    let resolved = match match_coached_member(data.repos(), requester, query, None).await? {
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
                thread_unreached(&refused, GateObject::Plan)
            })));
        }
    };

    if let Some(query) = query.filter(|query| !names_subject(&access.display_name, query)) {
        let name = &access.display_name;
        let message = match other_name(data.repos(), requester, query).await? {
            OtherName::Athlete(other) => format!(
                "This thread works on {name}'s plan — open {other}'s own thread from Group info \
                 to work on theirs."
            ),
            OtherName::Requester => format!(
                "This thread works on {name}'s plan — act on your own plan from your own \
                 conversation with me."
            ),
            OtherName::Nobody => format!(
                "This thread works on {name}'s plan, and '{query}' does not name them — omit \
                 `athlete` to act on {name}'s plan."
            ),
        };
        return Ok(Err(refusal(&message)));
    }

    acting_for(request, writer, access).await
}

/// What the model relays for a gate refusal about the athlete's plan.
/// `unreached` words the two that fail before the athlete is shown to be the
/// requester's — those name nobody.
fn gate_refusal(refused: &CoachedAthleteRefusal, unreached: impl FnOnce() -> String) -> ToolResult {
    refusal(&gate_message(refused, GateObject::Plan, unreached))
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
    // Every gate passed: the athlete's record names this call before it
    // reads or writes anything of theirs (carnet#1445). A plan tool reads the
    // athlete's plan and the training it is laid against, so the class is
    // their training; `get_training_plan` adds a wellness row when it reads
    // their recovery ([`PlanScope::record_wellness_read`]).
    record_coach_read(
        request.context.resources.repos(),
        &access,
        request.context.user_id,
        request.tool_name,
        CoachReadClass::Training,
    )
    .await?;
    // What the call reads and writes from here on is the athlete's
    // (carnet#1174).
    ai_scope::note_member_read(access.athlete_user_id);
    Ok(Ok(PlanScope {
        tenant: access.tenant,
        user_id: access.athlete_user_id,
        writer,
        subject: PlanSubject::Coached {
            access: Box::new(access),
        },
    }))
}
