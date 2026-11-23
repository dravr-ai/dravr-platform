// ABOUTME: The one usage row of a coaching turn an outside caller drove: an A2A agent's text message, the athlete's API key
// ABOUTME: A turn_summary row naming whose data it read and through what, the agent-turn entry of the owner feed (carnet#1543)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One `turn_summary` row per turn an outside caller drove.
//!
//! An A2A agent sending a text message, or the athlete's API key posting a
//! chat message, runs Dravr's own agent over the athlete's data and reads its
//! reply. The tools that agent calls are the agent's (they run on the turn's
//! own token and write no direct-tool-call row), so the read is recorded once
//! for the whole turn, here, where [`super::execute`] ends with the turn's
//! transport known: zero tokens (its LLM calls carry their own rows), the
//! turn's tools and end-to-end time, and the delegated columns the athlete's
//! "who read my data" feed lists it by. Dravr's own surfaces (web, mobile,
//! messaging) and the platform's own turns write nothing new. A slash
//! command an outside caller sends is recorded the same way, once its reply
//! is ready, and so is a turn that fails, with the tools it ran before it
//! did ([`TurnTools`](pierre_tool_runtime::turn_tools::TurnTools)): no read
//! appears nowhere.

use std::time::Instant;

use pierre_core::errors::AppResult;
use pierre_core::models::{CallOutcome, ConversationTurnId, DelegatedRead, InsertLlmUsage};
use pierre_core::permissions::impersonation::CallerCredential;
use pierre_core::transport::Transport;
use pierre_tool_runtime::context::SUPPORT_SESSION;
use pierre_tool_runtime::guardian::TurnKey;
use tracing::warn;

use super::{ServedTurn, TurnRequest};
use crate::ChatPipelineContext;

/// Whether a turn served over `transport` runs Dravr's agent for a caller
/// outside Dravr's own surfaces.
const fn drives_agent_for_outside_caller(transport: Transport) -> bool {
    match transport {
        Transport::A2a | Transport::ApiKey => true,
        Transport::WebApp
        | Transport::MobileApp
        | Transport::Messaging
        | Transport::PlatformJob
        | Transport::McpHttp => false,
    }
}

/// What a turn's row needs, taken before the turn runs.
pub(super) struct OutsideTurn {
    tenant_id: String,
    user_id: String,
    conversation_id: String,
    turn_id: ConversationTurnId,
    transport: Transport,
    caller: Option<CallerCredential>,
    impersonator: Option<String>,
    /// The key the turn's dispatches record the tools they ran under.
    turn_key: TurnKey,
    started: Instant,
}

/// How a served turn ends up in its row: its outcome and every tool it ran
/// that read data, in call order. The row's count is that list's length, so
/// the two never disagree.
///
/// - A served turn: `ok`, or `error` for a reply it did not store, with its
///   telemetry's tools: the agent's calls and the platform's prefetch of the
///   athlete's activities into the prompt alike (the tool loop's own count
///   leaves the prefetch out).
/// - A slash command's answer: `ok`, naming no tool, as its handler
///   dispatches none; it reads the athlete's account all the same.
/// - A turn that ended in an error: `error`, with `ran`, the tools the model
///   asked for before it failed.
fn turn_outcome<'a>(
    served: &'a AppResult<ServedTurn>,
    ran: &'a [String],
) -> (CallOutcome, &'a [String]) {
    match served {
        Ok(ServedTurn::Pipeline(envelope)) => {
            let outcome = if envelope.posted {
                CallOutcome::Ok
            } else {
                CallOutcome::Error
            };
            (outcome, &envelope.telemetry.tools_called)
        }
        Ok(ServedTurn::Command { .. }) => (CallOutcome::Ok, &[]),
        Err(_) => (CallOutcome::Error, ran),
    }
}

impl OutsideTurn {
    /// The turn `request` asks for, when an outside caller drove it.
    pub(super) fn of(request: &TurnRequest<'_>) -> Option<Self> {
        if !drives_agent_for_outside_caller(request.transport) {
            return None;
        }
        Some(Self {
            tenant_id: request.conversation_tenant_id.to_string(),
            user_id: request.user_id.to_string(),
            conversation_id: request.conversation_id.clone(),
            turn_id: request.turn_id,
            transport: request.transport,
            caller: request.caller.clone(),
            impersonator: SUPPORT_SESSION
                .try_with(|support| {
                    support
                        .as_ref()
                        .map(|grant| grant.impersonator_id.to_string())
                })
                .ok()
                .flatten(),
            turn_key: TurnKey::new(
                Some(request.tool_tenant_id.as_uuid()),
                request.turn_id.to_string(),
            ),
            started: Instant::now(),
        })
    }

    /// Write the turn's row from how it was `served`.
    ///
    /// The row's subject is the athlete the turn ran for. A failed write is
    /// logged and never fails the turn whose reply the caller is owed.
    pub(super) async fn record(self, ctx: &ChatPipelineContext, served: &AppResult<ServedTurn>) {
        // Taken whatever the turn's outcome, so no turn leaves its list behind.
        let ran = ctx.tool_runtime.turn_tools().take(&self.turn_key);
        let (outcome, tools) = turn_outcome(served, &ran);
        let tools_called = serde_json::to_string(tools).unwrap_or_else(|_| "[]".to_owned());
        let duration_ms = i64::try_from(self.started.elapsed().as_millis()).unwrap_or(i64::MAX);
        let usage = InsertLlmUsage::for_turn_summary(
            &self.tenant_id,
            &self.user_id,
            Some(&self.conversation_id),
            self.turn_id,
            &tools_called,
            i64::try_from(tools.len()).unwrap_or(i64::MAX),
            duration_ms,
        );
        let read = DelegatedRead {
            subject_user_id: Some(&self.user_id),
            client_id: self
                .caller
                .as_ref()
                .and_then(|caller| caller.client_id.as_deref()),
            credential: self.caller.as_ref().map(|caller| caller.kind.as_str()),
            transport: self.transport.as_str(),
            impersonator_user_id: self.impersonator.as_deref(),
            outcome,
        };
        if let Err(e) = ctx
            .repos
            .llm_usage
            .insert_delegated_usage(&usage, &read)
            .await
        {
            warn!(
                turn_id = %self.turn_id,
                "failed to record an outside caller's agent turn: {e}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::errors::{AppError, ErrorCode};

    use super::super::CommandTurn;
    use super::*;
    use crate::envelope::QuotaState;

    /// Each transport's answer, decided one by one: the match has no
    /// wildcard, so a transport added later fails to compile there until
    /// someone says whether its turns are an outside caller's.
    #[test]
    fn only_a2a_and_api_key_turns_are_driven_from_outside() {
        for transport in Transport::ALL {
            let (expected, why) = match transport {
                Transport::A2a => (true, "an A2A agent asks Dravr's agent"),
                Transport::ApiKey => (true, "an API key's caller asks Dravr's agent"),
                Transport::WebApp | Transport::MobileApp | Transport::Messaging => {
                    (false, "the athlete asks their own agent")
                }
                Transport::PlatformJob => (false, "the platform's own turn for the athlete"),
                Transport::McpHttp => (false, "an MCP client calls tools, each its own row"),
            };
            assert_eq!(
                drives_agent_for_outside_caller(transport),
                expected,
                "bug: {transport:?} misclassified ({why})"
            );
        }
    }

    #[test]
    fn a_turn_that_ended_in_an_error_is_an_error_row_naming_what_it_ran() {
        let ran = vec!["get_connection_status".to_owned()];
        for failed in [
            AppError::internal("boom"),
            AppError::new(ErrorCode::QuotaExceeded, "spent"),
        ] {
            assert_eq!(
                turn_outcome(&Err(failed), &ran),
                (CallOutcome::Error, &ran[..]),
                "bug: a failed turn is recorded, naming the tools it ran before failing"
            );
        }
    }

    #[test]
    fn a_slash_commands_answer_is_an_ok_turn_naming_no_tool() {
        let answered: AppResult<ServedTurn> = Ok(ServedTurn::Command {
            command: Box::new(CommandTurn {
                command_name: Some("status".to_owned()),
                text: "All good.".to_owned(),
                is_rich_text: false,
                card_title: None,
                actions: Vec::new(),
                rotated_to: None,
                opened_topic: None,
                persisted: None,
                welcome: None,
            }),
            quota: QuotaState::default(),
        });
        assert_eq!(
            turn_outcome(&answered, &["ignored".to_owned()]),
            (CallOutcome::Ok, &[][..]),
            "bug: a command an outside caller sent is a read the feed must list"
        );
    }
}
