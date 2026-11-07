// ABOUTME: Runs an A2A text message as one coaching turn, through the same turn service every chat surface enters
// ABOUTME: Holds what is the transport's: the instance's slot, the budget inside the HTTP door, the contextId mapping, the egress

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The A2A surface's ingress.
//!
//! Everything a turn *is* — the usage caps, the prompt, the provider terms, the
//! Guardian, the persona, the counters — lives in
//! [`pierre_chat_pipeline::execute`]. The turn is declared
//! [`Transport::A2a`], so every read it makes, prompt context and tool call
//! alike, withholds what a provider's terms keep on Dravr's own surfaces; it is
//! bound to the token's own grant, so it offers and runs nothing beyond it.
//!
//! The athlete is alone with their own agent, so the turn is a direct message:
//! it reads their plan as any 1:1 turn does. A hosted login scoped to the
//! athlete still never leaves over an external transport; the reconnect stage
//! gives the agent the linkless sentence.
//!
//! # Capacity
//!
//! A coaching turn holds the instance's serialized LLM runner for its whole
//! length: production runs one turn per pod across three pods
//! (`infra/environments/dev/terraform.tfvars`, the concurrency block). A public
//! caller must not be able to queue on that, so [`TURNS_PER_INSTANCE`] is a
//! `try_acquire` with no wait: a busy instance refuses at once and says to come
//! back. It bounds A2A's own turns on this instance. A first-party turn on the
//! same pod is not counted against it and does not count here.

use std::sync::Arc;

use pierre_a2a::coaching::CoachingTurns;

use crate::mcp::resources::ServerContext;

/// The A2A coaching seam for this build: `None` when it compiles no pipeline,
/// which refuses a text message.
#[cfg(all(feature = "client-chat", feature = "client-messaging"))]
#[must_use]
pub fn coaching_turns(resources: &Arc<ServerContext>) -> Option<Arc<dyn CoachingTurns>> {
    Some(Arc::new(PipelineCoachingTurns::new(Arc::clone(resources))))
}

/// The A2A coaching seam for this build: `None` when it compiles no pipeline,
/// which refuses a text message.
#[cfg(not(all(feature = "client-chat", feature = "client-messaging")))]
#[must_use]
pub fn coaching_turns(_resources: &Arc<ServerContext>) -> Option<Arc<dyn CoachingTurns>> {
    None
}

#[cfg(all(feature = "client-chat", feature = "client-messaging"))]
pub use pipeline::{PipelineCoachingTurns, TURNS_PER_INSTANCE};

#[cfg(all(feature = "client-chat", feature = "client-messaging"))]
mod pipeline {
    use std::fmt;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use pierre_a2a::coaching::{
        CanceledSignal, CoachingEnd, CoachingSlot, CoachingTurn, CoachingTurns,
    };
    use pierre_chat_pipeline::suggestions::is_postback;
    use pierre_chat_pipeline::turn_stop::TurnStop;
    use pierre_chat_pipeline::{
        check_pre_chat_quotas_scoped, execute, CommandPersistence, InputSource, PipelineHooks,
        PreChatScope, ReplyBlock, ServedTurn, SurfaceId, SurfaceProfile, SurfaceRequest,
        TurnOrigin, TurnRequest, A2A_TRANSPORT,
    };
    use pierre_core::errors::{AppError, AppResult, ErrorCode};
    use pierre_core::models::{ConversationRecord, ConversationTurnId, TenantId};
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_services::agent_selection::AgentSelectionSource;
    use pierre_services::conversation_forge::{
        forge_conversation, resolve_conversation_model, ForgeAgent, ForgeParams, SlotQuota,
    };
    use pierre_services::locale::resolve_user_locale;
    use tokio::select;
    use tokio::sync::Semaphore;
    use tracing::{error, warn};
    use uuid::Uuid;

    use crate::mcp::resources::ServerContext;
    use crate::services::conversation_turn_lock::{acquire_turn_lock, evict_idle_turn_lock};
    use crate::services::failed_turn_notice::{close_failed_turn, TurnThread};
    use crate::services::messaging_ingress::fresh_title_fallback;
    use crate::services::messaging_ingress::turn_guard::{
        run_bounded, run_guarded, TurnInterruption, TurnOutcome,
    };

    /// A2A coaching turns one instance runs at a time; see the module doc.
    pub const TURNS_PER_INSTANCE: usize = 1;

    /// The wall-clock ceiling of one turn: inside the HTTP door's 600 s, so a
    /// blocking `SendMessage` is answered before the door closes on it, and
    /// inside the task reaper's 10 minutes (`pierre_a2a::reaper::STALE_AFTER`),
    /// so a running turn is never failed as orphaned.
    const TURN_BUDGET: Duration = Duration::from_secs(540);

    /// The word a conversation and its rows carry for this surface.
    const CHANNEL: &str = SurfaceId::A2a.wire_channel();

    /// What a caller reads of any turn that ended without a reply. The reason
    /// is logged, never sent: it can name a model, a provider or a table.
    const TURN_FAILED: &str = "The coach could not answer this message. Try again shortly.";

    /// What a caller reads when the instance stopped under the turn.
    const TURN_INTERRUPTED: &str =
        "The coach was interrupted before answering. Send the message again.";

    /// What a caller reads of a `contextId` that cannot be theirs, the same
    /// whether it is malformed, another athlete's, or a thread of their own
    /// outside this surface: nothing here says which.
    const UNUSABLE_CONTEXT: &str =
        "This contextId cannot be used. Send the message without one to continue your conversation.";

    /// The production coaching seam, over the composition root.
    pub struct PipelineCoachingTurns {
        resources: Arc<ServerContext>,
        slots: Arc<Semaphore>,
    }

    impl fmt::Debug for PipelineCoachingTurns {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("PipelineCoachingTurns")
                .field("slots_free", &self.slots.available_permits())
                .finish_non_exhaustive()
        }
    }

    impl PipelineCoachingTurns {
        /// The seam over `resources`, with the instance's slots all free.
        #[must_use]
        pub fn new(resources: Arc<ServerContext>) -> Self {
            Self {
                resources,
                slots: Arc::new(Semaphore::new(TURNS_PER_INSTANCE)),
            }
        }

        /// Whether a conversation may carry this surface's turns: its own, on
        /// this surface, with nobody else in the room.
        fn is_a2a_thread(conversation: &ConversationRecord) -> bool {
            conversation.channel_type == CHANNEL
                && conversation.group_id.is_none()
                && conversation.subject().is_none()
        }

        /// Forge a conversation for this surface, under `id` when the client
        /// chose it.
        async fn forge(
            &self,
            tenant_id: TenantId,
            user_id: &str,
            id: Option<&str>,
        ) -> AppResult<String> {
            let resources = &self.resources;
            let title_fallback = fresh_title_fallback(resources, user_id).await;
            let model = resolve_conversation_model(None, &resources.common.config.llm_chain)?;
            let ctx = resources.chat_pipeline_context();
            forge_conversation(
                &resources.common.repos,
                ForgeParams {
                    id,
                    user_id,
                    tenant_id,
                    title_fallback: &title_fallback,
                    model: &model,
                    agent: ForgeAgent::Selected,
                    group_id: None,
                    channel_type: CHANNEL,
                    selection_source: AgentSelectionSource::ChatConversation,
                    // A walk asks the athlete questions; the reader is an agent.
                    guided_flow: false,
                    // A thread this surface opens takes a slot like any other;
                    // a client that names no context rolls on in one of them.
                    quota: SlotQuota::Capped(ctx.admin_config.as_deref()),
                },
            )
            .await
        }
    }

    /// The reply text a caller may read, or `None` when the turn left none.
    ///
    /// The last guard on the provider terms: the turn ran under
    /// [`Transport::A2a`], so nothing first-party-only should have reached it,
    /// and its stored row says what it actually derived from. A row stamped
    /// first-party-only stays on Dravr's own surfaces even if a filter upstream
    /// let something through.
    fn deliverable_reply(
        posted: bool,
        derived_from: TransportPolicy,
        blocks: &[ReplyBlock],
    ) -> Option<String> {
        if !posted {
            return None;
        }
        if derived_from.is_first_party_only() {
            error!("A2A text turn derived from first-party-only data; the reply is withheld");
            return None;
        }
        let prose: Vec<&str> = blocks
            .iter()
            .filter_map(|block| match block {
                ReplyBlock::Prose { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let reply = prose.join("\n\n");
        (!reply.trim().is_empty()).then_some(reply)
    }

    #[async_trait]
    impl CoachingTurns for PipelineCoachingTurns {
        fn try_reserve(&self) -> Option<CoachingSlot> {
            Arc::clone(&self.slots)
                .try_acquire_owned()
                .ok()
                .map(CoachingSlot::new)
        }

        fn is_postback(&self, text: &str) -> bool {
            is_postback(text)
        }

        async fn check_quota(&self, tenant_id: TenantId, user_id: Uuid) -> AppResult<()> {
            let ctx = self.resources.chat_pipeline_context();
            check_pre_chat_quotas_scoped(
                &ctx,
                tenant_id,
                user_id,
                &PreChatScope {
                    conversation_id: None,
                    agent_id: None,
                },
            )
            .await
            .map(|_| ())
        }

        async fn open_context(
            &self,
            tenant_id: TenantId,
            user_id: Uuid,
            context_id: Option<&str>,
        ) -> AppResult<String> {
            let user = user_id.to_string();
            let Some(raw) = context_id else {
                let rolling = self
                    .resources
                    .common
                    .repos
                    .chat
                    .latest_channel_thread(&user, tenant_id, CHANNEL)
                    .await?;
                return match rolling {
                    Some(thread) => Ok(thread.id),
                    None => self.forge(tenant_id, &user, None).await,
                };
            };
            let Ok(parsed) = Uuid::parse_str(raw) else {
                return Err(AppError::invalid_input(UNUSABLE_CONTEXT));
            };
            let id = parsed.to_string();
            let held = self
                .resources
                .common
                .repos
                .chat
                .get_conversation(&id, &user, tenant_id)
                .await?;
            match held {
                Some(conversation) if Self::is_a2a_thread(&conversation) => Ok(conversation.id),
                Some(_) => Err(AppError::invalid_input(UNUSABLE_CONTEXT)),
                // Nothing this athlete can read holds that id. A client may
                // choose its own ids, so the thread opens under it, unless a
                // row outside their reach holds it: then the insert conflicts,
                // and the refusal reads like any other.
                None => match self.forge(tenant_id, &user, Some(&id)).await {
                    Ok(forged) => Ok(forged),
                    Err(e) if e.code == ErrorCode::QuotaExceeded => Err(e),
                    Err(e) => {
                        warn!(error = %e, "A2A contextId could not open a conversation");
                        Err(AppError::invalid_input(UNUSABLE_CONTEXT))
                    }
                },
            }
        }

        async fn run(
            &self,
            turn: CoachingTurn,
            slot: CoachingSlot,
            canceled: CanceledSignal,
        ) -> Result<String, CoachingEnd> {
            let resources = &self.resources;
            let locale =
                resolve_user_locale(resources.common.repos.users.as_ref(), turn.user_id).await;
            let profile = SurfaceProfile::resolve(&SurfaceRequest {
                surface: SurfaceId::A2a,
                locale,
                transport: Some(A2A_TRANSPORT),
                prose_contract: None,
            });
            let ctx = resources.chat_pipeline_context();
            let stop = TurnStop::recording();
            let request = TurnRequest {
                conversation_id: turn.conversation_id.clone(),
                user_id: turn.user_id,
                conversation_tenant_id: turn.tenant_id,
                tool_tenant_id: turn.tenant_id,
                content: turn.content,
                // The athlete's agent speaks for them, and the thread is theirs.
                origin: TurnOrigin::Athlete,
                input_source: InputSource::Typed,
                turn_id: ConversationTurnId::new(),
                ambient_context: None,
                quote: None,
                channel_type: CHANNEL,
                is_direct_message: true,
                ambient_group_fallback: false,
                // Commands are refused before a turn opens.
                command_persistence: CommandPersistence::Always,
                sender_id: None,
                transport: Transport::A2a,
                grant: turn.grant,
                hooks: PipelineHooks {
                    stop: Some(Arc::clone(&stop)),
                    ..PipelineHooks::none()
                },
            };
            let drain = resources.common.turns.drain_token();
            let lock = acquire_turn_lock(&turn.conversation_id);
            let bounded = run_bounded(
                run_guarded(async {
                    let held = lock.lock().await;
                    let served = execute(&ctx, request, &profile).await;
                    drop(held);
                    served
                }),
                TURN_BUDGET,
                &drain,
            );
            let outcome = select! {
                biased;
                outcome = Box::pin(bounded) => Some(outcome),
                () = canceled => None,
            };

            let ended = match outcome {
                Some(TurnOutcome::Delivered(ServedTurn::Pipeline(envelope))) => deliverable_reply(
                    envelope.posted,
                    envelope.assistant.message.transport_policy,
                    &envelope.assistant.blocks,
                )
                .ok_or_else(|| CoachingEnd::Failed(TURN_FAILED.to_owned())),
                // Refused before a turn opens; a command that reached here
                // answered with something this surface does not offer.
                Some(TurnOutcome::Delivered(ServedTurn::Command { .. })) => {
                    Err(CoachingEnd::Failed(TURN_FAILED.to_owned()))
                }
                Some(TurnOutcome::QuotaDenied(e)) => {
                    Err(CoachingEnd::Failed(e.sanitized_message()))
                }
                Some(TurnOutcome::Failed(e)) => {
                    warn!(error = %e, "A2A text turn failed");
                    Err(CoachingEnd::Failed(TURN_FAILED.to_owned()))
                }
                Some(TurnOutcome::Interrupted(cause)) => {
                    warn!(
                        cause = TurnInterruption::as_str(cause),
                        "A2A text turn interrupted"
                    );
                    Err(CoachingEnd::Failed(TURN_INTERRUPTED.to_owned()))
                }
                None => Err(CoachingEnd::Canceled),
            };

            // A turn that stored the question and ended without a reply leaves
            // it open on a thread the athlete can read; close it, as the app's
            // and messaging's failed turns close theirs.
            if ended.is_err() {
                let held = lock.lock().await;
                close_failed_turn(
                    &TurnThread {
                        resources,
                        conversation_id: &turn.conversation_id,
                        user_id: turn.user_id.to_string(),
                        tenant_id: turn.tenant_id,
                    },
                    &stop,
                    &profile.locale,
                )
                .await;
                drop(held);
            }
            evict_idle_turn_lock(&turn.conversation_id, &lock);
            // Given back when the turn is over, however it ends: a canceled
            // or dropped turn drops the slot with its future.
            drop(slot);
            ended
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use pierre_a2a::reaper::STALE_AFTER;

        fn prose(text: &str) -> ReplyBlock {
            ReplyBlock::Prose {
                text: text.to_owned(),
            }
        }

        #[test]
        fn a_reply_derived_from_first_party_only_data_is_never_delivered() {
            let blocks = [prose("Your Nolio ride was hard.")];
            assert_eq!(
                deliverable_reply(true, TransportPolicy::FirstPartyOnly, &blocks),
                None,
                "bug: a reply stamped first-party-only must never leave over A2A"
            );
            assert_eq!(
                deliverable_reply(true, TransportPolicy::AnyTransport, &blocks).as_deref(),
                Some("Your Nolio ride was hard."),
                "bug: a reply any transport may carry is delivered as its prose"
            );
        }

        #[test]
        fn a_reply_that_was_not_stored_or_holds_no_prose_is_not_delivered() {
            let blocks = [prose("A draft.")];
            assert_eq!(
                deliverable_reply(false, TransportPolicy::AnyTransport, &blocks),
                None,
                "bug: a reply the turn did not store is not delivered"
            );
            assert_eq!(
                deliverable_reply(true, TransportPolicy::AnyTransport, &[prose("  ")]),
                None,
                "bug: a reply of blank prose is no reply"
            );
        }

        #[test]
        fn the_turn_budget_fits_the_door_and_the_reaper() {
            // 600 s is the HTTP door's request timeout, set outside this
            // codebase's constants: the frontend Cloud Run service's
            // `request_timeout` in infra/environments/dev/main.tf.
            assert!(
                TURN_BUDGET < Duration::from_secs(600),
                "bug: the turn budget must end before the HTTP door closes on a blocking send"
            );
            assert!(
                TURN_BUDGET < STALE_AFTER,
                "bug: the turn budget must end before the reaper fails a running task as orphaned"
            );
        }
    }
}
