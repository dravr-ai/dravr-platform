// ABOUTME: Whose calendar push_training_plan, prescribe_workout and withdraw_prescribed_workout write: the caller's own or a coached athlete's
// ABOUTME: The athlete= path runs on Dravr's own surfaces only, outside a room, through the plan scope, with the link's coach writer
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Calendar scope
//!
//! The three tools that write a training calendar write the caller's own by
//! default. A group's human coach names a coached athlete with `athlete=`
//! and writes that athlete's calendar instead, through the coach's own
//! account on the athlete's coaching platform. That path:
//!
//! 1. runs only on Dravr's own surfaces — its app and its channels. An MCP,
//!    A2A or API-key caller acting for a coach writes no athlete's calendar,
//!    whatever the plan's terms: it is another person's calendar.
//! 2. runs only from the coach's own conversation, never a group's room,
//!    where every member reads the reply;
//! 3. passes the plan scope's gates ([`resolve_plan_scope`]): the athlete is
//!    a member of a group the caller is the human coach of, shares their data
//!    with them, has one home tenant, and that tenant has the tool enabled.
//!    In the coach's thread bound to one athlete, that athlete is the one
//!    written for with no `athlete` named, and every rule here applies to them
//!    exactly as to a named one;
//! 4. writes through the link's own coach writer
//!    ([`AuthService::coach_calendar_writer`]): the caller's own link, the
//!    athlete's `calendar_write` consent, and Coach access on the platform.
//!
//! What a coach writes keeps the athlete's side whole: the calendar target
//! comes from the athlete's connections, "today" from the athlete's clock,
//! and the ledger, the plan and stored sessions live under the athlete's
//! tenant. The coach's side is the conversation the call came from and the
//! agent it answers as ([`PlanWriter`]).
//!
//! A refusal on this path is in-band, `{reason, error}`, in the coach's
//! words; nothing on the athlete's calendar is read or written before it, and
//! nothing is saved among the athlete's sessions.

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppResult, ErrorCode};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::core::FitnessProvider;
use pierre_tools_core::ToolResult;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use super::calendar::{
    calendar_provider, calendar_target, holds_only_through_a_link, CalendarConnections,
};
use super::plan_scope::{resolve_plan_scope, PlanScopeRequest, PlanWriter};
use super::training_plan_authorship::load_conversation;
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;

/// The `athlete` property the three calendar-write tools share.
pub(super) fn athlete_property() -> PropertySchema {
    PropertySchema {
        property_type: "string".to_owned(),
        description: Some(
            "Roster display name of the athlete whose calendar this writes. Only the group's \
             human coach (attached via a coach invite) may set it, for a consenting athlete in a \
             group they coach whose coaching-platform link allows calendar writes, from Dravr's \
             own app or channels and from a direct chat — never in a room. In the coach's thread \
             about one athlete it defaults to that athlete; elsewhere, omit it to write your own \
             calendar."
                .to_owned(),
        ),
        ..Default::default()
    }
}

/// A refusal in the tool's error shape, with a machine-readable reason.
pub(super) fn refusal(reason: &str, message: &str) -> ToolResult {
    ToolResult::error(json!({ "reason": reason, "error": message }))
}

/// The refusal of an athlete's own write to their calendar on `backend`,
/// which they read only through a human coach's link: only that coach writes
/// it, from Dravr, and only while the athlete allows calendar writes on the
/// link. Connecting the platform themselves would end the link, so the
/// refusal never asks them to.
fn human_coach_writes(context: &ToolExecutionContext, backend: &str) -> ToolResult {
    let brand = context
        .resources
        .provider_registry()
        .get_display_name(backend)
        .unwrap_or(backend);
    refusal(
        "human_coach_writes_calendar",
        &format!(
            "Your {brand} calendar is read through your human coach's link, so only that coach \
             writes it, from Dravr, once you allow calendar writes on the link in the group. \
             Ask them to make the change. Connecting {brand} yourself would end that link."
        ),
    )
}

/// The refusal for a call made outside the caller's own conversation, if it
/// was: a group's room, or a conversation that does not resolve as theirs.
/// `act` says what to do from their own conversation instead.
///
/// # Errors
///
/// Returns the repository error from the conversation lookup.
#[cfg(feature = "tools-groups")]
pub(super) async fn room_refusal(
    context: &ToolExecutionContext,
    tenant: TenantId,
    act: &str,
) -> AppResult<Option<ToolResult>> {
    if context.conversation_id.is_none() {
        return Ok(None);
    }
    let conversation = load_conversation(
        context.resources.repos(),
        context.conversation_id.as_deref(),
        tenant,
        &context.user_id.to_string(),
    )
    .await?;
    Ok(room_refusal_in(context, conversation.as_ref(), act))
}

/// [`room_refusal`] for a call whose conversation, read under the caller's
/// own tenant, is `conversation`.
pub(super) fn room_refusal_in(
    context: &ToolExecutionContext,
    conversation: Option<&ConversationRecord>,
    act: &str,
) -> Option<ToolResult> {
    // A conversation the caller's own tenant cannot resolve is a shared
    // room's row under the bot tenant: refused like a group-bound one.
    let in_room = context.conversation_id.is_some()
        && conversation.is_none_or(|conversation| conversation.group_id.is_some());
    in_room.then(|| {
        refusal(
            "group_room",
            &format!(
                "{act} from your own conversation with Dravr — a group's room is read by every \
                 member."
            ),
        )
    })
}

/// Whose calendar a calendar-write tool writes, and who is writing.
pub(super) struct CalendarScope {
    /// Tenant the athlete's calendar ledger, plan and stored sessions live
    /// under.
    pub(super) tenant: TenantId,
    /// The athlete whose calendar it is.
    pub(super) user_id: Uuid,
    /// The requester and the agent their turn answers as.
    pub(super) writer: PlanWriter,
    /// The athlete's display name on the coach path; `None` in self scope.
    acting_for: Option<String>,
    /// The link the coach's writer goes through, once it is built.
    link_id: Option<Uuid>,
    /// When the coach's credential was read for the writer.
    credential_read_at: Option<DateTime<Utc>>,
}

/// Resolve whose calendar the call writes: the caller's own, or — for a
/// group's human coach naming `athlete`, or calling from their thread bound
/// to one athlete, from their own conversation on Dravr's own surfaces — a
/// coached athlete's.
///
/// `arg_agent` is the call's `agent_id` argument, read only on a call with
/// no conversation and no turn agent. The outer `Err` is a repository
/// failure; the inner `Err` a refusal the tool returns as its result.
///
/// # Errors
///
/// Returns an error when the call carries no tenant, or a repository error
/// from a lookup the resolution needs.
pub(super) async fn resolve_calendar_scope(
    context: &ToolExecutionContext,
    args: &Value,
    arg_agent: Option<String>,
    tool_name: &str,
) -> AppResult<Result<CalendarScope, ToolResult>> {
    let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
    let athlete = args
        .get("athlete")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let conversation = load_conversation(
        context.resources.repos(),
        context.conversation_id.as_deref(),
        requester_tenant,
        &context.user_id.to_string(),
    )
    .await?;
    // A thread bound to one athlete writes that athlete's calendar whether or
    // not the call names them, so it counts exactly as `athlete=` does.
    let for_athlete = athlete.is_some()
        || conversation
            .as_ref()
            .and_then(ConversationRecord::subject)
            .is_some();
    // Another person's calendar is written only from Dravr's own surfaces:
    // refused before anything about the athlete is read.
    if for_athlete && ai_scope::serving_external() {
        return Ok(Err(refusal(
            "external_transport",
            "A coach writes an athlete's calendar from Dravr's own app or channels, not over \
             this interface.",
        )));
    }
    if for_athlete {
        if let Some(refused) = room_refusal_in(
            context,
            conversation.as_ref(),
            "Write an athlete's calendar",
        ) {
            return Ok(Err(refused));
        }
    }
    let scope = match resolve_plan_scope(PlanScopeRequest {
        context,
        requester_tenant,
        conversation: conversation.as_ref(),
        arg_agent,
        athlete,
        tool_name,
    })
    .await?
    {
        Ok(scope) => scope,
        Err(refused) => return Ok(Err(refused)),
    };
    Ok(Ok(CalendarScope {
        tenant: scope.tenant,
        user_id: scope.user_id,
        writer: scope.writer,
        acting_for: scope.acting_for,
        link_id: None,
        credential_read_at: None,
    }))
}

impl CalendarScope {
    /// Whether a group's human coach writes a coached athlete's calendar,
    /// rather than the caller their own.
    pub(super) const fn acts_for_athlete(&self) -> bool {
        self.acting_for.is_some()
    }

    /// The athlete's display name when a coach writes their calendar; `None`
    /// in self scope.
    pub(super) fn athlete_name(&self) -> Option<&str> {
        self.acting_for.as_deref()
    }

    /// The backend this scope's calendar write goes to: the one `requested`
    /// names, else the athlete's connected calendar. A coach writes the
    /// calendar the athlete's link reads.
    ///
    /// In self scope the athlete's own connections are chosen among first. An
    /// athlete who holds none and names none still has their write aimed at
    /// the calendar they read through a human coach's link, so that
    /// [`Self::calendar_writer`] refuses it in that coach's terms rather than
    /// asking them to connect a calendar, which would end the link. A named
    /// provider is resolved whoever's connection holds it: the writer is
    /// where that is checked.
    ///
    /// # Errors
    ///
    /// Returns the refusal [`calendar_target`] gives.
    pub(super) async fn target(
        &self,
        context: &ToolExecutionContext,
        requested: Option<&str>,
    ) -> AppResult<String> {
        if self.acts_for_athlete() {
            return calendar_target(
                context,
                self.tenant,
                self.user_id,
                requested,
                CalendarConnections::WithLink,
            )
            .await;
        }
        match calendar_target(
            context,
            self.tenant,
            self.user_id,
            requested,
            CalendarConnections::Own,
        )
        .await
        {
            Err(own) if requested.is_none() && own.code == ErrorCode::NoProviderConnected => {
                calendar_target(
                    context,
                    self.tenant,
                    self.user_id,
                    None,
                    CalendarConnections::WithLink,
                )
                .await
            }
            resolved => resolved,
        }
    }

    /// The provider this scope's calendar on `backend` is written with: the
    /// athlete's own in self scope, the link's coach writer on the coach
    /// path, or the refusal either path answers with.
    ///
    /// Every self-scope write reaches its calendar here, however its backend
    /// was chosen (named, the ledger row it replaces or withdraws, or the
    /// athlete's connections), so a calendar the athlete holds only through a
    /// human coach's link is refused here, once, before its provider is
    /// built: only that coach writes it.
    ///
    /// # Errors
    ///
    /// Returns the self-scope refusal of an athlete who holds no connection
    /// to `backend`, a repository error, or the error of building the coach's
    /// writer.
    pub(super) async fn calendar_writer(
        &mut self,
        context: &ToolExecutionContext,
        backend: &str,
    ) -> AppResult<Result<Box<dyn FitnessProvider>, ToolResult>> {
        if !self.acts_for_athlete() {
            if holds_only_through_a_link(context, self.tenant, self.user_id, backend).await? {
                return Ok(Err(human_coach_writes(context, backend)));
            }
            return Ok(Ok(calendar_provider(
                context,
                self.tenant,
                self.user_id,
                backend,
            )
            .await?));
        }
        self.credential_read_at = Some(Utc::now());
        match AuthService::new(context.resources.clone())
            .coach_calendar_writer(self.writer.user_id, self.user_id, self.tenant, backend)
            .await?
        {
            Ok(writer) => {
                self.link_id = Some(writer.link_id);
                Ok(Ok(writer.provider))
            }
            Err(refused) => Ok(Err(refusal(refused.reason, &refused.message))),
        }
    }

    /// `result`, a write or read of the athlete's calendar through
    /// `provider`, once its failure is reacted to as a read through the link
    /// reacts: a dead coach credential flags the coach's connection, an
    /// athlete the platform no longer lets the coach read ends the link. A
    /// refused write ends nothing. Nothing is reacted to in self scope.
    pub(super) async fn reacting<T>(
        &self,
        context: &ToolExecutionContext,
        provider: &dyn FitnessProvider,
        result: AppResult<T>,
    ) -> AppResult<T> {
        if let (Some(started), Err(error)) = (self.credential_read_at, &result) {
            AuthService::new(context.resources.clone())
                .react_to_delegated_refusal(
                    self.user_id,
                    &self.tenant.to_string(),
                    provider,
                    error,
                    started,
                )
                .await;
        }
        result
    }

    /// Log the `rows` ledger rows `tool` wrote on the coach path, by the link
    /// and the coach who wrote them. Nothing is logged in self scope.
    pub(super) fn record_coach_write(&self, tool: &str, rows: usize) {
        if let Some(link_id) = self.link_id {
            info!(
                link_id = %link_id,
                coach_user_id = %self.writer.user_id,
                tool,
                rows,
                "Coach wrote a coached athlete's calendar ledger"
            );
        }
    }
}
