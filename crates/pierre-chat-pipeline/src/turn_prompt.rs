// ABOUTME: Stage 2 of a turn: store the prompt it answers, then refuse it while the LLM is down
// ABOUTME: Also names the message the turn answers, which every tool it dispatches reads as TURN_ASK
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{ConversationTurnId, TurnOrigin};
use pierre_database::database::MessageRecord;
use pierre_llm::health::LlmHealthStatus;
use pierre_tool_runtime::context::TurnAsk;
use tracing::warn;

use crate::context::ChatPipelineContext;
use crate::hooks::PipelineHooks;
use crate::stages::persistence::resolve_turn_prompt;
use crate::surface_profile::SurfaceProfile;
use crate::turn::{TurnInput, UserMessageResult};
use crate::turn_stop;

/// Stage 2: resolve the prompt the turn answers, record it on the turn's stop
/// handle, then refuse the turn while the LLM probe reads the provider down.
///
/// Refused only once the question is on file: a turn the provider outage stops
/// still leaves the sender's words on the thread, for the surface to close with
/// its notice rather than to lose (carnet#1362).
///
/// # Errors
///
/// Returns the prompt resolution's error, or
/// [`ErrorCode::ResourceUnavailable`] from [`refuse_when_llm_unhealthy`].
pub(crate) async fn store_turn_prompt(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    profile: &SurfaceProfile,
    hooks: &PipelineHooks<'_>,
) -> AppResult<UserMessageResult> {
    let (groups, channel) = (ctx.repos.groups.as_ref(), profile.surface.wire_channel());
    let msg_result = resolve_turn_prompt(ctx.repos.chat.as_ref(), groups, input, channel).await?;
    turn_stop::question_stored(hooks, &msg_result.message);
    refuse_when_llm_unhealthy(ctx).await?;
    Ok(msg_result)
}

/// The turn `input` is and the message it answers, for the tools it dispatches.
///
/// Only an athlete's own prompt is a stored message: a prompt the platform
/// composed is never written as the athlete's row (`resolve_turn_prompt`), so
/// it names none, and a read the turn starts answers no message of theirs.
pub(crate) fn turn_ask(input: &TurnInput, message: &MessageRecord) -> TurnAsk {
    ask_of(input.turn_id, input.origin, &message.id)
}

fn ask_of(turn_id: ConversationTurnId, origin: TurnOrigin, message_id: &str) -> TurnAsk {
    TurnAsk {
        turn_id: turn_id.0.to_string(),
        user_message_id: origin.persists_user_row().then(|| message_id.to_owned()),
    }
}

/// Fail the turn fast with a 503 while the LLM health probe reads
/// `Unhealthy`, rather than sending it to a provider known to be down.
///
/// # Errors
///
/// Returns [`ErrorCode::ResourceUnavailable`] naming the provider the probe
/// found down.
async fn refuse_when_llm_unhealthy(ctx: &ChatPipelineContext) -> AppResult<()> {
    let snapshot = ctx.llm_health.snapshot().await;
    if !matches!(snapshot.status, LlmHealthStatus::Unhealthy) {
        return Ok(());
    }
    let provider = snapshot.provider.unwrap_or_else(|| "llm".to_owned());
    let last_error = snapshot.error.unwrap_or_else(|| "unknown".to_owned());
    warn!(
        provider = %provider,
        last_error = %last_error,
        "LLM probe is Unhealthy — failing chat turn fast with 503"
    );
    Err(AppError::new(
        ErrorCode::ResourceUnavailable,
        format!(
            "Our AI provider ({provider}) is temporarily unavailable. We've been notified — please try again in a minute or two."
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn an_athletes_turn_names_the_message_it_answers() {
        let turn = ConversationTurnId(Uuid::new_v4());
        let ask = ask_of(turn, TurnOrigin::Athlete, "msg-1");
        assert_eq!(
            ask,
            TurnAsk {
                turn_id: turn.0.to_string(),
                user_message_id: Some("msg-1".to_owned()),
            },
            "an athlete's turn carries the turn and its stored message to its tools"
        );
    }

    #[test]
    fn a_platform_composed_turn_names_no_message() {
        let ask = ask_of(
            ConversationTurnId(Uuid::new_v4()),
            TurnOrigin::Platform,
            "draft",
        );
        assert_eq!(
            ask.user_message_id, None,
            "a prompt the platform composed is not a message the athlete asked"
        );
    }
}
