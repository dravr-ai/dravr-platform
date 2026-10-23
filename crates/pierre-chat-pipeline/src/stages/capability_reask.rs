// ABOUTME: The repair re-asks capability recovery sends — with verified data, with no provider, with an unreadable one
// ABOUTME: Each re-ask's reply replaces the draft only when it answers, and claims of unreadable data only where true
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The re-ask half of [`super::capability_recovery`]: one repair completion
//! over the turn, carrying platform-fetched evidence and an instruction that
//! fits what the verification fetch found, and the rule for whether its reply
//! replaces the draft.

use pierre_core::errors::AppError;
use pierre_core::narration;
use pierre_llm::call_record::complete_recorded;
use pierre_llm::{ChatRequest, ChatResponse, FunctionResponse};
use pierre_services::chat_provider_factory::chat_provider_from_resources_arc;
use pierre_tool_runtime::protocol::provider_helpers::NO_PROVIDER_CONNECTED_TEXT;
use pierre_tool_runtime::protocol::META_NO_PROVIDER_CONNECTED;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use pierre_tool_runtime::tool_results::format_tool_results_as_text;
use serde_json::json;
use tracing::{info, warn};

use super::capability_recovery::{
    repair_messages, turn_language_directive, CapabilityRecoveryDeps, REASK_INSTRUCTION,
    VERIFICATION_TOOL,
};

/// The re-ask instruction when the athlete has no provider connected and the
/// draft answered a data question without a tool: any figure in it was made
/// up, since there is nothing to read it from.
const NO_DATA_REASK_INSTRUCTION: &str = "The athlete has no fitness provider connected, so \
     there is no activity or health data for them, and your draft above was not grounded in \
     any. Replace it with your reply to the athlete's last message: answer what general \
     knowledge can answer, never state a figure, date or session that is not in the athlete's \
     own messages, and say plainly that connecting Strava, Garmin or another provider lets you \
     use their own training.";

/// The re-ask instruction when the turn came back empty and the athlete's data
/// source could not be read: the source exists but failed, so the model says
/// so and answers what it can without it.
const UNAVAILABLE_REASK_INSTRUCTION: &str = "The athlete's activity data could not be read \
     just now: their provider is connected but did not answer. Your draft above came back \
     empty. Reply to the athlete's last message: say plainly that their data could not be \
     loaded right now and that they can try again shortly, answer what general knowledge can \
     answer, and never state a figure, date or session that is not in the athlete's own \
     messages.";

/// One completion over the turn's messages plus the verified tool result,
/// taken only if it comes back free of capability-failure claims. Returns the
/// tool result as the text the model saw, so a re-check of the replacement
/// can count it as evidence.
pub(super) async fn reask_with_verified_data(
    deps: &CapabilityRecoveryDeps<'_>,
    payload: serde_json::Value,
    result: &mut ToolLoopResult,
) -> String {
    reask_with(
        deps,
        payload,
        REASK_INSTRUCTION,
        ClaimPolicy::Refuted,
        result,
    )
    .await
}

/// Re-ask a draft that answered a data question for an athlete with no
/// provider connected, telling the model there is no data to answer from.
pub(super) async fn reask_without_data(
    deps: &CapabilityRecoveryDeps<'_>,
    result: &mut ToolLoopResult,
) {
    info!(
        "capability_recovery: no provider connected; re-asking the ungrounded draft without data"
    );
    let payload = json!({
        "error": NO_PROVIDER_CONNECTED_TEXT,
        META_NO_PROVIDER_CONNECTED: true,
    });
    reask_with(
        deps,
        payload,
        NO_DATA_REASK_INSTRUCTION,
        ClaimPolicy::True,
        result,
    )
    .await;
}

/// Whether a re-ask may say the athlete's data cannot be read.
#[derive(Clone, Copy)]
enum ClaimPolicy {
    /// The data was just fetched: such a claim is false, and a re-ask still
    /// making it is not taken.
    Refuted,
    /// Nothing is connected: "I can't see your training" is simply true.
    True,
}

/// Re-ask a turn that came back empty while the athlete's data source failed
/// to answer, so they get a reply saying so instead of nothing.
pub(super) async fn reask_while_unavailable(
    deps: &CapabilityRecoveryDeps<'_>,
    result: &mut ToolLoopResult,
) {
    info!("capability_recovery: empty reply and an unreadable data source; re-asking without data");
    let payload =
        json!({ "error": "The athlete's provider did not answer; no activity data was read." });
    reask_with(
        deps,
        payload,
        UNAVAILABLE_REASK_INSTRUCTION,
        ClaimPolicy::True,
        result,
    )
    .await;
}

/// One repair completion over the turn plus `payload` as platform-fetched tool
/// output and `instruction`; the replacement is taken when it answers, and
/// under [`ClaimPolicy::Refuted`] only when it is also free of
/// capability-failure claims. Returns the tool output as the model saw it.
async fn reask_with(
    deps: &CapabilityRecoveryDeps<'_>,
    payload: serde_json::Value,
    instruction: &str,
    policy: ClaimPolicy,
    result: &mut ToolLoopResult,
) -> String {
    let function_response = FunctionResponse {
        name: VERIFICATION_TOOL.to_owned(),
        response: payload,
    };
    let tool_text = format_tool_results_as_text(&[function_response]);

    let Ok(provider) = chat_provider_from_resources_arc(
        deps.ctx.chat_provider.as_ref(),
        deps.ctx.llm_provider.as_ref(),
    ) else {
        // A missing provider handle is a wiring bug, not a transient condition
        // (see `resolve_reask_provider`); the fetched data still proves access
        // works, but without a completion there is nothing to re-ask with.
        warn!(
            "capability recovery found no provider for the re-ask; leaving the reply as delivered"
        );
        return tool_text;
    };

    let messages = repair_messages(
        deps.llm_messages,
        &result.content,
        &format!(
            "{tool_text}\n\n{instruction}\n\n{}",
            turn_language_directive(deps)
        ),
    );
    let request = ChatRequest::new(messages).with_model(deps.active_model);

    apply_reask_outcome(
        complete_recorded(provider.as_ref(), &request, Some(deps.reask_recorder)).await,
        policy,
        result,
    );
    tool_text
}

/// Take the re-ask's reply if it is free of capability-failure claims;
/// otherwise leave the original reply alone. Mirrors the identity re-ask's
/// accept-only-if-clean contract.
fn apply_reask_outcome(
    outcome: Result<ChatResponse, AppError>,
    policy: ClaimPolicy,
    result: &mut ToolLoopResult,
) {
    match outcome {
        Ok(reply) => apply_reask_reply(reply, policy, result),
        Err(e) => warn!(
            error = %e,
            "capability_failure_reask_failed: re-ask did not complete; \
             keeping the original reply"
        ),
    }
}

/// The completed re-ask: taken when [`reask_verdict`] says it is usable.
fn apply_reask_reply(reply: ChatResponse, policy: ClaimPolicy, result: &mut ToolLoopResult) {
    match reask_verdict(&reply.content, policy) {
        ReaskVerdict::Usable => {
            info!(
                reply_len = reply.content.len(),
                "capability_failure_reask_recovered: re-ask with verified data \
                 produced a usable reply; the athlete keeps their turn"
            );
            result.content = reply.content;
        }
        ReaskVerdict::NoAnswer => warn!(
            reply_len = reply.content.len(),
            "capability_failure_reask_degenerate: re-ask carried no answer; \
             keeping the original reply"
        ),
        ReaskVerdict::StillClaims => warn!(
            "capability_failure_reask_persisted: re-ask still claimed broken \
             access with the data in hand; keeping the original reply"
        ),
    }
}

/// Whether a re-ask's reply may replace the original.
enum ReaskVerdict {
    /// Substantive and free of capability-failure claims.
    Usable,
    /// Empty or a dangling fragment — a lost turn, not a corrected reply.
    NoAnswer,
    /// Still claims broken access with the data in hand.
    StillClaims,
}

/// Classify a re-ask's reply — see [`ReaskVerdict`].
fn reask_verdict(content: &str, policy: ClaimPolicy) -> ReaskVerdict {
    if narration::is_degenerate_reply(content) {
        ReaskVerdict::NoAnswer
    } else if matches!(policy, ClaimPolicy::Refuted)
        && narration::contains_capability_failure(content)
    {
        ReaskVerdict::StillClaims
    } else {
        ReaskVerdict::Usable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With nothing connected, "I can't see your training" is the true answer
    /// a no-data re-ask should give: it is taken there, while a re-ask made
    /// with the data in hand still may not claim access is broken.
    #[test]
    fn a_no_data_reask_may_say_the_data_cannot_be_read() {
        let honest = "I can't access your data: no provider is connected yet. \
                      Connect Strava or Garmin and I can read your training.";
        assert!(
            narration::contains_capability_failure(honest),
            "the fixture must carry capability-failure vocabulary to test anything"
        );
        assert!(
            matches!(
                reask_verdict(honest, ClaimPolicy::True),
                ReaskVerdict::Usable
            ),
            "with nothing connected, saying the data cannot be read is the answer"
        );
        assert!(
            matches!(
                reask_verdict(honest, ClaimPolicy::Refuted),
                ReaskVerdict::StillClaims
            ),
            "with the data in hand, a claim that it cannot be read is still refused"
        );
    }
}
