// ABOUTME: A2A text messages: who may ask the coach, what it costs them, and how the reply reaches the task
// ABOUTME: Every refusal runs before a task opens, so a refused message leaves nothing behind and spends nothing

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coaching text skill.
//!
//! A `text` part that addresses no tool runs one coaching turn for the athlete
//! the token acts for. The turn itself is the pipeline's, entered where every
//! chat surface enters it (`pierre_chat_pipeline::execute`), so the provider
//! terms, the Guardian, the persona and the usage caps are the ones every other
//! surface answers to. What this module decides is the protocol's half: who
//! may start one, and what a task says about it.
//!
//! An orchestrator holding a client-credentials token acts for no athlete, so
//! it has no coach to ask. A slash command is account control, which no third
//! party is offered. A turn reads the athlete's data and profile into its
//! prompt whatever tool it calls, so a grant missing either read cannot be
//! narrowed to what it holds and is refused whole.

use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::TenantId;
use pierre_mcp_transport::tenant_isolation::{extract_tenant_context_internal, log_tenant_failure};
use pierre_runtime_context::A2ACtx;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use tokio::sync::oneshot;
use tokio::time;
use tracing::{error, warn};

use super::principal::AuthPrincipal;
use super::{A2AResources, A2AServer, PreparedSend, SendFailure, SendOutcome};
use crate::agent_card::COACHING_SKILL_SCOPES;
use crate::coaching::{CanceledSignal, CoachingEnd, CoachingSlot, CoachingTurn, CoachingTurns};
use crate::protocol_types::{A2ASpecError, Message, PartContent};
use crate::A2AResponse;

/// The name of the artifact a coaching reply lands in.
pub(super) const REPLY_ARTIFACT_NAME: &str = "Coach reply";

/// The longest message a text turn reads. No athlete types this much; a caller
/// that sends more is spending the athlete's token budget on a prompt.
const MAX_TEXT_TURN_CHARS: usize = 16_000;

/// The wait a caller is told to leave when the instance has no place free. A
/// turn takes tens of seconds, and another instance may have one sooner.
const NO_SLOT_RETRY_AFTER_SECS: u64 = 30;

/// The wait a spent cap reports when it names no reset time.
const QUOTA_RETRY_AFTER_SECS: u64 = 60;

/// How often a running turn re-reads its task for a cancel.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// What a client-credentials caller reads: it acts for no athlete.
const NO_ATHLETE_MESSAGE: &str =
    "A text message asks an athlete's coach, and a client-credentials token acts for no athlete. \
     Use a grant an athlete authorized, or address a tool with a data part.";

/// What a caller reads for a slash command or a suggestion's postback.
const COMMAND_MESSAGE: &str =
    "Commands are not available over A2A: send the athlete's question as plain text.";

/// A text message admitted to a coaching turn.
pub(super) struct AdmittedTextTurn {
    turn: CoachingTurn,
    slot: CoachingSlot,
    coaching: Arc<dyn CoachingTurns>,
}

/// The text a message carries for the coach: every `text` part, joined. `None`
/// when it holds none, or only blanks.
fn message_text(message: &Message) -> Option<String> {
    let parts: Vec<&str> = message
        .parts
        .iter()
        .filter_map(|part| match &part.content {
            PartContent::Text(text) => Some(text.trim()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Whether the text would reach the turn's command dispatch rather than the
/// coach: a slash command, or the postback of a tapped suggestion.
fn is_command_text(text: &str, is_postback: impl Fn(&str) -> bool) -> bool {
    let text = text.trim();
    text.starts_with('/') || is_postback(text)
}

impl A2AServer {
    /// Admit a text message to a coaching turn, or refuse it before any task
    /// opens. Stamps the conversation the turn runs in onto the message as its
    /// `contextId`. `None` when the message is no text turn: a tool intent, a
    /// continuation of an existing task, or nothing to read.
    pub(super) async fn admit_text_turn(
        resources: &A2AResources,
        principal: &AuthPrincipal,
        message: &mut Message,
        request_id: Option<&Value>,
    ) -> Result<Option<AdmittedTextTurn>, Box<A2AResponse>> {
        if Self::extract_tool_intent(message).is_some() || message.task_id.is_some() {
            return Ok(None);
        }
        let Some(content) = message_text(message) else {
            return Ok(None);
        };
        let refuse = |error: A2ASpecError, text: String| {
            Box::new(Self::spec_error_with_message(
                error,
                text,
                request_id.cloned(),
            ))
        };

        // A build with no pipeline has nothing to run a text message in: it
        // reads as a message addressing no tool.
        let Some(coaching) = &resources.coaching else {
            return Ok(None);
        };
        if principal.client_id.is_some() {
            return Err(refuse(
                A2ASpecError::UnsupportedOperation,
                NO_ATHLETE_MESSAGE.to_owned(),
            ));
        }
        if let Some(missing) = COACHING_SKILL_SCOPES
            .iter()
            .find(|scope| !principal.scopes.contains(scope))
        {
            return Err(refuse(
                A2ASpecError::UnsupportedOperation,
                format!(
                    "A text message needs the {missing} scope, which this grant does not hold."
                ),
            ));
        }
        if is_command_text(&content, |text| coaching.is_postback(text)) {
            return Err(refuse(
                A2ASpecError::ContentTypeNotSupported,
                COMMAND_MESSAGE.to_owned(),
            ));
        }
        if content.chars().count() > MAX_TEXT_TURN_CHARS {
            return Err(Box::new(Self::invalid_params(
                format!("A text message holds at most {MAX_TEXT_TURN_CHARS} characters."),
                request_id.cloned(),
            )));
        }

        let tenant_id = Self::text_turn_tenant(resources, principal, request_id).await?;
        let Some(slot) = coaching.try_reserve() else {
            return Err(Box::new(Self::rate_limited_error(
                "The coach is busy. Try again shortly.",
                NO_SLOT_RETRY_AFTER_SECS,
                request_id.cloned(),
            )));
        };
        coaching
            .check_quota(tenant_id, principal.user_id)
            .await
            .map_err(|e| Box::new(Self::coaching_refusal(&e, request_id)))?;
        let conversation_id = coaching
            .open_context(tenant_id, principal.user_id, message.context_id.as_deref())
            .await
            .map_err(|e| Box::new(Self::coaching_refusal(&e, request_id)))?;

        message.context_id = Some(conversation_id.clone()); // Safe: String ownership for the task record
        Ok(Some(AdmittedTextTurn {
            turn: CoachingTurn {
                user_id: principal.user_id,
                tenant_id,
                conversation_id,
                content,
                grant: principal.scopes.clone(),
            },
            slot,
            coaching: Arc::clone(coaching),
        }))
    }

    /// The tenant the token's grant acts in, its membership checked as a tool
    /// call's is.
    async fn text_turn_tenant(
        resources: &A2AResources,
        principal: &AuthPrincipal,
        request_id: Option<&Value>,
    ) -> Result<TenantId, Box<A2AResponse>> {
        let tenant = principal.active_tenant_id.map(TenantId::from_uuid);
        match extract_tenant_context_internal(resources.ctx.repos(), principal.user_id, tenant)
            .await
        {
            Ok(Some(context)) => Ok(context.tenant_id),
            Ok(None) => Err(Box::new(Self::invalid_params(
                "User does not belong to any tenant",
                request_id.cloned(),
            ))),
            Err(e) => {
                log_tenant_failure(principal.user_id, &e);
                Err(Box::new(Self::a2a_error(
                    -32000,
                    "Failed to resolve tenant context",
                    request_id.cloned(),
                )))
            }
        }
    }

    /// The answer to a refusal the pipeline's own checks gave: a spent cap
    /// carries its wait, a bad `contextId` is the caller's to fix, and anything
    /// else says nothing about the server.
    fn coaching_refusal(error: &AppError, request_id: Option<&Value>) -> A2AResponse {
        match error.code {
            ErrorCode::QuotaExceeded | ErrorCode::RateLimitExceeded => Self::rate_limited_error(
                error.sanitized_message(),
                error.retry_after_secs().unwrap_or(QUOTA_RETRY_AFTER_SECS),
                request_id.cloned(),
            ),
            ErrorCode::InvalidInput => {
                Self::invalid_params(error.sanitized_message(), request_id.cloned())
            }
            _ => {
                error!("A2A text turn could not be admitted: {error}");
                Self::a2a_error(
                    -32000,
                    "The coach could not take this message",
                    request_id.cloned(),
                )
            }
        }
    }

    /// Run a prepared message detached from the request and wait for it.
    ///
    /// A blocking `SendMessage` holds its connection for the whole turn, and a
    /// client that drops it drops this future. Run inline, that would drop the
    /// turn mid-way and leave the task `working` until the reaper; detached,
    /// the turn finishes and `GetTask` reports it. Spawned under the server's
    /// in-flight tracker, as a background send is.
    pub(super) async fn run_detached_and_wait(
        ctx: &Arc<dyn A2ACtx>,
        tool_runtime: &Arc<dyn ToolRuntime>,
        principal: AuthPrincipal,
        prepared: PreparedSend,
    ) {
        let (finished, wait) = oneshot::channel::<()>();
        let (run_ctx, run_runtime) = (Arc::clone(ctx), Arc::clone(tool_runtime));
        ctx.spawn_detached_task(Box::pin(async move {
            Self::run_task_message(&run_ctx, &run_runtime, principal, prepared).await;
            // The waiter is gone when the client dropped; the task says the rest.
            if finished.send(()).is_err() {
                warn!("A2A text turn finished after its caller left");
            }
        }));
        // An error means the run panicked past its boundary: the task row says
        // where it stands, which is what the caller reads next.
        if wait.await.is_err() {
            warn!("A2A text turn ended without reporting back");
        }
    }

    /// Run an admitted turn to the outcome the task records.
    pub(super) async fn run_text_turn(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        admitted: AdmittedTextTurn,
    ) -> SendOutcome {
        let AdmittedTextTurn {
            turn,
            slot,
            coaching,
        } = admitted;
        match coaching
            .run(turn, slot, Self::canceled_signal(ctx, task_id))
            .await
        {
            Ok(reply) => SendOutcome::Coach(reply),
            Err(CoachingEnd::Canceled) => SendOutcome::Canceled,
            Err(CoachingEnd::Failed(reason)) => SendOutcome::Failure(SendFailure::text(reason)),
        }
    }

    /// Resolves once the task reaches a terminal state someone else wrote: a
    /// `CancelTask`, on whichever instance took it, or the reaper.
    fn canceled_signal(ctx: &Arc<dyn A2ACtx>, task_id: &str) -> CanceledSignal {
        let (ctx, task_id) = (Arc::clone(ctx), task_id.to_owned());
        Box::pin(async move {
            loop {
                time::sleep(CANCEL_POLL_INTERVAL).await;
                match ctx.repos().a2a.get_task(&task_id).await {
                    Ok(Some(task)) if !task.status.is_terminal() => {}
                    Ok(_) => return,
                    // A read that failed says nothing about the task.
                    Err(e) => warn!(error = %e, "A2A text turn could not re-read its task"),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol_types::Part;

    fn message(parts: Vec<Part>) -> Message {
        Message::agent("m".to_owned(), parts)
    }

    #[test]
    fn text_parts_join_and_blanks_read_as_no_text() {
        let two = message(vec![
            Part::text(" how was my week? ".to_owned()),
            Part::text("and tomorrow?".to_owned()),
        ]);
        assert_eq!(
            message_text(&two).as_deref(),
            Some("how was my week?\nand tomorrow?"),
            "bug: every text part reaches the coach, trimmed and joined by a newline"
        );
        assert_eq!(
            message_text(&message(vec![Part::text("  ".to_owned())])),
            None,
            "bug: a message of blanks holds no text for the coach"
        );
        assert_eq!(
            message_text(&message(vec![Part::data(serde_json::json!({"a": 1}))])),
            None,
            "bug: a data part is no text for the coach"
        );
    }

    #[test]
    fn a_command_is_told_apart_from_a_question() {
        let never = |_: &str| false;
        assert!(
            is_command_text(" /logout", never),
            "bug: a slash command reads as a command, leading blanks and all"
        );
        assert!(
            is_command_text("use_case:1:abc", |text| text.starts_with("use_case:")),
            "bug: a suggestion's postback reads as a command"
        );
        assert!(
            !is_command_text("how far is 10/2 km?", never),
            "bug: a slash inside a question does not make it a command"
        );
    }
}
