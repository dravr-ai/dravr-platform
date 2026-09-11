// ABOUTME: Handler for /fortnight — the platform reads the plan and decides whether the next two weeks land
// ABOUTME: Gather and decide happen here in Rust; the drafting, and the readiness check, are the agent's

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_FORTNIGHT_ARM_FAILED, KEY_FORTNIGHT_BLOCKED, KEY_FORTNIGHT_COVERED,
    KEY_FORTNIGHT_NO_PHASES, KEY_FORTNIGHT_NO_PLAN, KEY_FORTNIGHT_WALK_RUNNING,
    KEY_FORTNIGHT_WRITING,
};
use pierre_core::errors::AppError;
use pierre_core::models::onboarding::{GuidedFlow, OnboardingState};
use pierre_database::repositories::training_plans::PlanOwner;
use pierre_messaging::commands::CommandResponse;
use pierre_services::athlete_clock::athlete_today;
use pierre_services::fortnight::{
    decide_fortnight, CoverageReading, DeclineReason, FortnightInputs, FortnightVerdict,
    WalkReading, FORTNIGHT_WEEKS,
};
use pierre_services::training_plan_render::{resolve_plan_agent_slug, select_active_weeks};
use tracing::{info, warn};

use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/fortnight` — write the next two weeks of the plan.
///
/// The gather and the decide are the platform's, in Rust: whether there is a
/// plan, whether it states phases, and whether the stored weeks already run
/// past the fortnight. The drafting is the agent's, because what a Tuesday
/// should be is a coaching judgement the platform has no business making.
///
/// The readiness ladder is deliberately *not* read here. Reading it costs the
/// rails' whole gather — the training history, the recovery feed, the sleep
/// feed and the agent package — which the agent asks for with `include_state`
/// on the turn this opens, and which it needs anyway to write the days. The
/// decision therefore sees `None` for readiness, and `None` is silence: it
/// refuses nothing on that ground rather than treating an unread ladder as
/// consent.
///
/// The go-ahead is not the end of the rail. It leaves a retired
/// [`GuidedFlow::Fortnight`] marker on the conversation, and the turn after it
/// carries the brief that names the gather, the two weeks and the save — the
/// only thing that makes the reply's promise true. It is written before the
/// promise is said, so a conversation that acquired a walk in between is
/// refused rather than told a draft is coming.
///
/// Every refusal names what would change it. A "no" without a next step is
/// what sends an agent inventing one.
pub struct FortnightHandler;

#[async_trait]
impl CommandHandler for FortnightHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let repos = ctx.ctx.repos();
        let today = athlete_today(repos, ctx.user_id).await;

        // One read of the conversation answers both questions: whose plan
        // this is, and whether a guided walk already owns the turn after this
        // one — the turn this command's go-ahead is written for.
        let conversation = match ctx.conversation_id.as_deref() {
            Some(cid) => repos
                .chat
                .get_conversation(cid, &ctx.user_id.to_string(), ctx.conversation_tenant_id)
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "/fortnight could not read the conversation");
                    None
                }),
            None => None,
        };
        let onboarding_state = conversation
            .as_ref()
            .and_then(|c| c.onboarding_state.clone());
        let walk = if OnboardingState::from_column(onboarding_state.as_deref()).is_some() {
            WalkReading::Running
        } else {
            WalkReading::Idle
        };
        let conversation_agent = conversation.and_then(|c| c.agent_id);
        let agent =
            resolve_plan_agent_slug(repos, conversation_agent, ctx.tenant_id, ctx.user_id).await?;

        let plan = repos
            .training_plans
            .get_active_plan(
                &ctx.tenant_id.to_string(),
                &ctx.user_id.to_string(),
                PlanOwner::from_slug(agent.as_deref()),
            )
            .await
            .unwrap_or_else(|e| {
                // An unreadable plan is not an absent one, but the athlete
                // gets the same honest answer either way: there is nothing
                // here to extend right now.
                warn!(error = %e, "/fortnight could not read the active plan");
                None
            });

        let inputs = match plan.as_ref() {
            Some(plan) => {
                let weeks = repos
                    .training_plans
                    .list_plan_weeks(
                        &ctx.tenant_id.to_string(),
                        &ctx.user_id.to_string(),
                        &plan.id,
                        false,
                    )
                    .await
                    .unwrap_or_else(|e| {
                        warn!(error = %e, "/fortnight could not read the plan weeks");
                        Vec::new()
                    });
                Some(FortnightInputs {
                    has_phases: !plan.phases.is_empty(),
                    // The ladder is not read here, and the message does not
                    // pretend it was. Reading it needs the rails' full
                    // gather — three history stores and the agent package —
                    // which is the agent's to ask for with `include_state`
                    // on the turn this opens. `None` is silence, and the
                    // decision treats it as such rather than as consent.
                    readiness: None,
                    // Covered means the fortnight the athlete acts on is
                    // already written: the same selection the card and the
                    // prompt block make, so all three agree about which two
                    // weeks "the fortnight" is.
                    coverage: if select_active_weeks(&weeks, today, FORTNIGHT_WEEKS)
                        .weeks
                        .len()
                        >= FORTNIGHT_WEEKS
                    {
                        CoverageReading::Covered
                    } else {
                        CoverageReading::RunningOut
                    },
                })
            }
            None => None,
        };

        let verdict = decide_fortnight(walk, inputs.as_ref());
        info!(
            user_id = %ctx.user_id,
            verdict = ?verdict,
            "/fortnight decided",
        );

        let text = match verdict {
            // The go-ahead is only said once the brief that keeps it is
            // stored, and what to say when it is not depends on why. A
            // conversation that acquired a walk between the read above and
            // this write is told about the walk; anything else is a failure
            // to arm the turn, and saying "you are mid-walk" to an athlete
            // who is not would be a plainer lie than saying nothing.
            FortnightVerdict::Write { weeks } => {
                match open_rail(ctx, onboarding_state.as_deref()).await {
                    RailOutcome::Opened => {
                        reg.render(KEY_FORTNIGHT_WRITING, &ctx.locale, &[&weeks.to_string()])
                    }
                    RailOutcome::WalkTookTheTurn => {
                        reg.render(KEY_FORTNIGHT_WALK_RUNNING, &ctx.locale, &[])
                    }
                    RailOutcome::NotOpened => {
                        reg.render(KEY_FORTNIGHT_ARM_FAILED, &ctx.locale, &[])
                    }
                }
            }
            FortnightVerdict::Decline { reason } => {
                reg.render(decline_key(reason), &ctx.locale, &[])
            }
        };
        Ok(CommandResponse::text(text))
    }
}

/// Open the [`GuidedFlow::Fortnight`] rail on this conversation, and say what
/// became of it.
///
/// The command answers the athlete and ends; the drafting happens on the turn
/// after, and the flow is what tells that turn what it is for. It stays
/// ACTIVE rather than retiring immediately, because the fortnight is not
/// finished when it is drafted — the athlete reads two weeks and says "make
/// Thursday easier", and that turn belongs to the rail that computed the
/// brief rather than to ordinary coaching with the brief scrolled out of
/// reach. The rail retires itself after
/// [`FORTNIGHT_FLOW_TURNS`](pierre_chat_pipeline) turns.
///
/// Compare-and-set against the column this turn read, for the reason
/// `/calibrate` does the same: a walk started under a running turn owns the
/// column by now, and a blind write would delete the interview the athlete is
/// in the middle of.
///
/// A column holding another flow's *retired* marker is overwritten, and that
/// is deliberate. It means a walk had just ended and its release directive
/// was waiting — the season wrap-up's offer to lay out the season, say — and
/// the athlete typed `/fortnight` rather than answering it. The newest thing
/// they asked for owns the turn they get next.
async fn open_rail(ctx: &PlatformCommandContext, expected: Option<&str>) -> RailOutcome {
    let Some(conversation_id) = ctx.conversation_id.as_deref() else {
        // No conversation, no next turn to brief — a channel that cannot
        // carry the drafting must not be told the drafting is coming.
        return RailOutcome::NotOpened;
    };
    let marker = OnboardingState::start_now_column(GuidedFlow::Fortnight);
    match ctx
        .ctx
        .repos()
        .chat
        .compare_and_set_conversation_onboarding_state(
            conversation_id,
            expected,
            Some(&marker),
            ctx.conversation_tenant_id,
        )
        .await
    {
        Ok(true) => RailOutcome::Opened,
        Ok(false) => {
            // The compare-and-set matched nothing, which for this column
            // means a flow started under the turn that read it.
            warn!(
                conversation_id = %conversation_id,
                "a newer guided flow owns this conversation; /fortnight did not open its rail"
            );
            RailOutcome::WalkTookTheTurn
        }
        Err(e) => {
            warn!(error = %e, "/fortnight could not open its rail");
            RailOutcome::NotOpened
        }
    }
}

/// What became of the rail the go-ahead depends on.
///
/// Three outcomes rather than a bool because the athlete reads a different
/// sentence for each, and two of them are refusals of the same go-ahead for
/// entirely different reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RailOutcome {
    /// Opened; the next turn belongs to the rail.
    Opened,
    /// A guided flow claimed the conversation while this command ran.
    WalkTookTheTurn,
    /// The rail could not be opened at all — no conversation to open it on,
    /// or the write failed. Nothing about the athlete's state, so nothing
    /// about their state is said.
    NotOpened,
}

/// The string a refusal renders.
const fn decline_key(reason: DeclineReason) -> &'static str {
    match reason {
        DeclineReason::WalkRunning => KEY_FORTNIGHT_WALK_RUNNING,
        DeclineReason::NoPlan => KEY_FORTNIGHT_NO_PLAN,
        DeclineReason::NoPhases => KEY_FORTNIGHT_NO_PHASES,
        DeclineReason::AlreadyCovered => KEY_FORTNIGHT_COVERED,
        DeclineReason::ReadinessBlocked => KEY_FORTNIGHT_BLOCKED,
    }
}
