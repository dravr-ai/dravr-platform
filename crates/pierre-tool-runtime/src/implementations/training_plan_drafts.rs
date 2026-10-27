// ABOUTME: A human coach's draft of a coached athlete's plan on the tool path — the save into it, the read of it, and publish_training_plan
// ABOUTME: Also the publish entry point the subject-plan route shares: the service's publish, then the rails every live plan write runs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's draft, on the tool path (carnet#1069)
//!
//! When a group's human coach acts for a coached athlete
//! ([`PlanScope::acting_for`]), `save_training_plan` writes the coach's
//! draft of the athlete's plan rather than the season the athlete reads
//! ([`save_into_draft`]), `get_training_plan` returns the draft beside the
//! live plan ([`draft_section`]), and `publish_training_plan` makes the
//! draft live or throws it away ([`PublishTrainingPlanTool`]).
//!
//! Publishing goes through [`publish_draft_live`], which the subject-plan
//! route behind the coach's Publish button calls too: the service's publish
//! ([`publish_coach_draft`]) — goal fact, one transaction, the
//! `training_plan.published` record, the athlete's `coach_plan_published`
//! notice — then the rails every live plan write runs
//! ([`run_live_plan_rails`]). Neither path can publish without the other's
//! steps.

use crate::annotations::{annotations, Effect, Reach};
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::repositories::training_plans::PlanAuthor;
use pierre_database::repositories::{PlanOutlineInput, PlanWeekInput, SaveDraftParams};
use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::PropertySchema;
use pierre_memory::training_plans::PlanWeek;
use pierre_providers::ai_scope;
use pierre_services::coach_plan_draft::{
    discard_coach_draft, draft_over, publish_coach_draft, CoachDraftPublish, CoachDraftTarget,
    DraftState, PublishedCoachDraft,
};
use pierre_services::plan_fueling::FuelingDisclosure;
use pierre_tools_core::ToolResult;
use serde_json::{json, Value};
use tracing::info;

use super::plan_scope::{resolve_plan_scope, PlanScope, PlanScopeRequest};
use super::training_plan_authorship::load_conversation;
use super::training_plan_push::calendar_preview_after_save;
use super::training_plan_rails::{draft_plan_safety, run_live_plan_rails, LivePlanWrite};
use super::training_plans_output::{
    race_summary, saved_weeks, DraftSection, PlanSafetyReport, PublishStatus,
    PublishTrainingPlanResult, SaveTrainingPlanResult,
};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;

/// The tool's name, as the scope check and the log lines name it.
const PUBLISH_TOOL: &str = "publish_training_plan";

/// A coach's save, validated, on its way into their draft.
pub(super) struct DraftSave<'a> {
    /// Whose draft.
    pub(super) target: &'a CoachDraftTarget,
    /// The athlete's roster name, for the reply.
    pub(super) athlete: &'a str,
    /// The agent the coach's turn answers as.
    pub(super) author: PlanAuthor<'a>,
    /// A goal fact of the athlete's the payload named and the save verified.
    pub(super) goal_fact_id: Option<&'a str>,
    /// The outline, when the save re-lays one.
    pub(super) outline: Option<PlanOutlineInput<'a>>,
    /// The weeks.
    pub(super) weeks: &'a [PlanWeekInput<'a>],
    /// Argument keys the tool does not declare, reported back.
    pub(super) ignored_arguments: Vec<String>,
}

/// Write a coach's save into their draft of the athlete's plan.
///
/// Nothing the athlete reads changes: not their season, not their goal fact,
/// not their calendar, and nothing is reported as a write to their plan. The
/// reply says so, and names the tool that makes the draft live. It carries
/// the two safety-shaped findings on the draft view ([`draft_plan_safety`]),
/// so the coach learns of a ramp spike or crowded hard days while the draft
/// is still theirs to change.
///
/// # Errors
///
/// Returns the repository error from the save, and invalid input for a
/// weeks-only save with no season to draft them over.
pub(super) async fn save_into_draft(
    state: &Arc<dyn ToolRuntime>,
    repos: &RepositoryRegistry,
    save: DraftSave<'_>,
) -> AppResult<ToolResult> {
    let target = save.target;
    let laid_outline = save.outline.is_some();
    let saved = repos
        .training_plans
        .save_draft_bundle(&SaveDraftParams {
            owner: target.owner(),
            author: save.author,
            goal_fact_id: save.goal_fact_id,
            outline: save.outline,
            weeks: save.weeks,
            transport_policy: ai_scope::derived_policy(),
        })
        .await?;
    let draft = &saved.draft.plan;
    info!(
        athlete_user_id = %target.athlete(),
        coach_user_id = %target.coach(),
        draft_id = %draft.id,
        opened = saved.opened,
        outline_drafted = saved.draft.outline_drafted,
        weeks_saved = saved.weeks.len(),
        "coach saved into their draft of a coached athlete's plan"
    );
    let safety = draft_plan_safety(state, repos, target, &saved, laid_outline).await;
    let athlete = save.athlete;
    let weeks = saved_weeks(&saved.weeks);
    ok_typed(
        "save_training_plan",
        SaveTrainingPlanResult {
            safety: (!safety.is_empty()).then_some(safety),
            ignored_arguments: save.ignored_arguments,
            plan_id: draft.id.clone(),
            athlete: Some(athlete.to_owned()),
            goal_race: race_summary(&draft.goal_race),
            superseded_plan_id: None,
            goal_fact_id: draft.goal_fact_id.clone(),
            weeks_saved: weeks.len(),
            weeks,
            calendar: None,
            draft: true,
            message: Some(format!(
                "Saved to your draft of {athlete}'s plan. {athlete} does not see it until you \
                 publish it with publish_training_plan (action \"publish\"); discard it with \
                 action \"discard\"."
            )),
        },
    )
}

/// The coach's draft of the athlete's plan for `get_training_plan`: its
/// outline, the live weeks `live_weeks` with the drafted ones in their place,
/// and where it stands. `None` outside the coach path, and when the coach
/// holds no draft.
///
/// # Errors
///
/// Returns the repository error from the draft reads, and
/// `UnavailableOverTransport` for a draft an external caller may not read.
pub(super) async fn draft_section(
    repos: &RepositoryRegistry,
    scope: &PlanScope,
    live_weeks: &[PlanWeek],
    fueling: &FuelingDisclosure,
) -> AppResult<Option<DraftSection>> {
    if scope.acting_for.is_none() {
        return Ok(None);
    }
    let target = CoachDraftTarget::new(scope.tenant, scope.user_id, scope.writer.user_id);
    let Some(mut view) = draft_over(repos, &target, live_weeks).await? else {
        return Ok(None);
    };
    let fueling_withheld = fueling.redact_weeks(&mut view.weeks);
    Ok(Some(DraftSection {
        plan: view.plan,
        weeks: view.weeks,
        drafted_weeks: view.drafted_weeks,
        state: view.state,
        fueling_withheld,
    }))
}

/// A published draft, with what the rails found in it.
#[derive(Debug)]
pub struct PublishedDraftReport {
    /// What the publish wrote.
    pub published: PublishedCoachDraft,
    /// The two safety-shaped findings of the rails, empty when none.
    pub safety: PlanSafetyReport,
    /// The athlete's civil date the rails measured against.
    pub today: NaiveDate,
}

/// What [`publish_draft_live`] came to.
#[derive(Debug)]
pub enum DraftPublishOutcome {
    /// The draft is the athlete's live plan.
    Published(Box<PublishedDraftReport>),
    /// The athlete's plan moved under the draft and the coach did not ask to
    /// overwrite it; nothing was written.
    Conflict(DraftState),
    /// The coach holds no open draft of this athlete's plan.
    NoDraft,
}

/// Publish the coach's draft as the athlete's live plan, tell the athlete,
/// then run the rails.
///
/// The rails are the ones every live plan write runs, over what this one
/// wrote. The athlete is told through the runtime's notification service,
/// when it has one ([`PublishedCoachDraft::athlete_notified`]). This is the
/// entry point both `publish_training_plan` and the subject-plan publish
/// route call; the caller has run the coaching gates for `target`.
///
/// # Errors
///
/// The errors of [`publish_coach_draft`].
pub async fn publish_draft_live(
    state: &Arc<dyn ToolRuntime>,
    target: &CoachDraftTarget,
    overwrite: bool,
) -> AppResult<DraftPublishOutcome> {
    let repos = state.repos();
    #[cfg(feature = "client-notifications")]
    let notifications = state.notification_service();
    #[cfg(not(feature = "client-notifications"))]
    let notifications = None;
    let published = match publish_coach_draft(repos, notifications, target, overwrite).await? {
        CoachDraftPublish::Published(published) => *published,
        CoachDraftPublish::Conflict(standing) => {
            return Ok(DraftPublishOutcome::Conflict(standing))
        }
        CoachDraftPublish::NoDraft => return Ok(DraftPublishOutcome::NoDraft),
    };
    let rails = run_live_plan_rails(
        state,
        repos,
        &LivePlanWrite {
            tenant: target.tenant(),
            user_id: target.athlete(),
            plan: &published.plan,
            weeks: &published.weeks,
            laid_outline: published.outline_published,
            flavour: published
                .outline_published
                .then_some(published.plan.flavour.as_ref())
                .flatten(),
        },
    )
    .await;
    Ok(DraftPublishOutcome::Published(Box::new(
        PublishedDraftReport {
            published,
            safety: rails.safety,
            today: rails.today,
        },
    )))
}

/// What the coach asks of their draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftAction {
    Publish,
    Discard,
}

impl DraftAction {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim) {
            Some("publish") => Some(Self::Publish),
            Some("discard") => Some(Self::Discard),
            _ => None,
        }
    }
}

/// A refusal in the tool's error shape, with the code a client branches on.
fn refusal(reason: &str, message: &str) -> ToolResult {
    ToolResult::error(json!({ "reason": reason, "error": message }))
}

/// `publish_training_plan` — a coach's draft of a coached athlete's plan made
/// live, or thrown away.
#[derive(Debug)]
pub struct PublishTrainingPlanTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for PublishTrainingPlanTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "athlete".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Roster display name of the coached athlete whose plan your draft is of. Only \
                     the group's human coach (attached via a coach invite) may set it, for a \
                     consenting athlete in a group they coach, and only from a direct chat — \
                     never in a room. In the coach's thread about one athlete it defaults to that \
                     athlete."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "action".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "`publish` makes your draft the athlete's live plan; `discard` throws it \
                     away and leaves their plan as it is. One of: publish, discard."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "overwrite_athlete_changes".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some(
                    "Publish over the weeks the athlete's own agent changed since you drafted \
                     them, and over a season re-laid under your draft. Set it only after the \
                     coach saw the conflicts and chose to replace the athlete's changes. Read \
                     only with `publish`."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["action".to_owned()]));
        // Named by literal: the catalogue checks read the tool's name off
        // this call, never a constant.
        answers_with::<PublishTrainingPlanResult>(tool_definition(
            "publish_training_plan",
            "Publish or discard a group's human coach's draft of a coached athlete's training \
             plan. A coach's save_training_plan for an athlete writes the coach's draft, which \
             the athlete does not see: publish makes it their live plan in one step — the weeks \
             the coach did not draft keep the athlete's own version — and discard throws it \
             away. Call it only when the coach asks to publish or discard. When the athlete's \
             own agent changed a drafted week after the coach drafted it, or re-laid the \
             season, publish is refused with status `conflict` and nothing changes: tell the \
             coach which weeks, and publish again with overwrite_athlete_changes only if they \
             choose to replace the athlete's changes. Publishing does not touch the athlete's \
             calendar; push_training_plan does that afterwards. In a coach's thread about one \
             athlete, `athlete` defaults to that athlete. The caller's own plan has no draft: \
             their saves are already live.",
            schema,
            annotations(Effect::Destructive, Reach::OpenWorld),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let requester_tenant = TenantId::from_uuid(context.require_tenant()?);
            let Some(action) = DraftAction::parse(args.get("action").and_then(Value::as_str))
            else {
                return Ok(refusal(
                    "invalid_action",
                    "action must be \"publish\" or \"discard\".",
                ));
            };
            let overwrite = args
                .get("overwrite_athlete_changes")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let athlete = args
                .get("athlete")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty());
            let repos = context.resources.repos();
            let conversation = load_conversation(
                repos,
                context.conversation_id.as_deref(),
                requester_tenant,
                &context.user_id.to_string(),
            )
            .await?;
            let scope = match resolve_plan_scope(PlanScopeRequest {
                context: &context,
                requester_tenant,
                conversation: conversation.as_ref(),
                arg_agent: None,
                athlete,
                tool_name: PUBLISH_TOOL,
            })
            .await?
            {
                Ok(scope) => scope,
                Err(refused) => return Ok(refused),
            };
            let Some(name) = scope.acting_for.clone() else {
                return Ok(refusal(
                    "own_plan",
                    "Your own saves are already live — there is no draft of your own plan to \
                     publish. publish_training_plan publishes a group coach's draft of a coached \
                     athlete's plan.",
                ));
            };
            let target = CoachDraftTarget::new(scope.tenant, scope.user_id, scope.writer.user_id);
            match action {
                DraftAction::Discard => discard(repos, &target, name).await,
                DraftAction::Publish => {
                    publish(state, &context, &scope, &target, name, overwrite).await
                }
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

/// The refusal when the coach holds no draft of `name`'s plan.
fn no_draft(name: &str) -> ToolResult {
    refusal(
        "no_draft",
        &format!(
            "You have no draft of {name}'s plan — save_training_plan writes one, and it stays \
             yours until you publish or discard it."
        ),
    )
}

async fn discard(
    repos: &RepositoryRegistry,
    target: &CoachDraftTarget,
    name: String,
) -> AppResult<ToolResult> {
    let Some(draft_id) = discard_coach_draft(repos, target).await? else {
        return Ok(no_draft(&name));
    };
    let message = format!(
        "Your draft of {name}'s plan is discarded. Their plan is as it was, and they were told \
         nothing."
    );
    ok_typed(
        PUBLISH_TOOL,
        PublishTrainingPlanResult::new(PublishStatus::Discarded, name, draft_id, message),
    )
}

async fn publish(
    state: &Arc<dyn ToolRuntime>,
    context: &ToolExecutionContext,
    scope: &PlanScope,
    target: &CoachDraftTarget,
    name: String,
    overwrite: bool,
) -> AppResult<ToolResult> {
    let report = match publish_draft_live(state, target, overwrite).await? {
        DraftPublishOutcome::Published(report) => report,
        DraftPublishOutcome::NoDraft => return Ok(no_draft(&name)),
        DraftPublishOutcome::Conflict(standing) => {
            let message = format!(
                "Nothing was published: {name}'s own plan changed under your draft. Tell the \
                 coach which weeks (and whether the season was re-laid), and publish again with \
                 overwrite_athlete_changes only if they choose to replace {name}'s changes."
            );
            let draft_id = standing.draft_id.clone();
            return ok_typed(
                PUBLISH_TOOL,
                PublishTrainingPlanResult {
                    conflict: Some(standing),
                    ..PublishTrainingPlanResult::new(
                        PublishStatus::Conflict,
                        name,
                        draft_id,
                        message,
                    )
                },
            );
        }
    };
    let PublishedDraftReport {
        published,
        safety,
        today,
    } = *report;
    // Publishing does not write the calendar; when the calendar already
    // carries this plan, say what a push would now change.
    let fueling =
        FuelingDisclosure::for_athlete(state.repos(), scope.tenant, scope.user_id).await?;
    let calendar =
        calendar_preview_after_save(context, scope, &published.plan.id, today, &fueling).await;
    let told = if published.athlete_notified {
        format!("{name} was sent a notice that it is published.")
    } else {
        format!("No notice reached {name}: let them know it is published.")
    };
    let message = format!(
        "Your draft is {name}'s live plan now. The weeks you did not draft keep {name}'s own \
         version. {told}"
    );
    ok_typed(
        PUBLISH_TOOL,
        PublishTrainingPlanResult {
            plan_id: Some(published.plan.id),
            superseded_plan_id: published.superseded_plan_id,
            weeks_published: published.weeks.into_iter().map(|w| w.week_start).collect(),
            athlete_notified: Some(published.athlete_notified),
            safety: (!safety.is_empty()).then_some(safety),
            calendar,
            ..PublishTrainingPlanResult::new(
                PublishStatus::Published,
                name,
                published.draft_id,
                message,
            )
        },
    )
}

/// Factory for the draft publish tool.
#[must_use]
pub fn create_training_plan_publish_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![Box::new(PublishTrainingPlanTool)]
}

// Publishing replaces the athlete's live season and weeks — the athlete's own
// changes too, under overwrite — and discarding throws the coach's draft
// away; neither can be undone from here. Irreversible, so at most one per
// turn under the Guardian's default budget, as push_training_plan is, and
// the confirm path applies to it when armed.
crate::declare_security!(PublishTrainingPlanTool => IRREVERSIBLE);
