// ABOUTME: AG-UI run wiring + per-channel status-bridge setup for messaging dispatch
// ABOUTME: Carries the RunScope/BroadcastSink plus the optional StatusAdapter consumer task

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_core::models::messaging::ChannelConfig;
use pierre_messaging::agui_status::StatusAdapter;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::agui::{AgUiEventFilter, BroadcastSink, RunOwner, RunScope};
use crate::contremaitre::messaging_strings::KEY_THINKING_PLACEHOLDER;
use crate::services::chat_pipeline;
use crate::services::messaging_status_bridge::{
    open_status_adapter, spawn_status_consumer, OpenStatusParams,
};

use super::PendingDispatch;

/// AG-UI wiring for a single messaging turn.
///
/// Holds the [`RunScope`] (auto-unregisters on drop) plus the
/// [`BroadcastSink`] the pipeline emits events through.
///
/// Optionally carries a per-channel `StatusAdapter` + a background
/// consumer task handle.
///
/// When the inbound channel supports in-place progress updates
/// (Telegram/Slack/Discord), the status bridge opens a placeholder
/// message and spawns a task that mirrors AG-UI events as
/// `editMessageText` / `chat.update` / `PATCH messages`.
///
/// On turn completion the caller calls [`Self::finalize_reply`] to
/// collapse the placeholder into the final assistant reply.
pub(super) struct MessagingAgUiWiring {
    scope: RunScope,
    sink: BroadcastSink,
    thread_id: String,
    status_adapter: Option<Arc<dyn StatusAdapter + Send + Sync>>,
    /// Abort handle for the AG-UI → status adapter consumer task.
    ///
    /// The task normally terminates on its own when the broadcast
    /// closes (i.e. when `scope` drops); the handle is retained so
    /// early errors can stop it promptly rather than wait for the
    /// close event to propagate.
    status_consumer: Option<JoinHandle<()>>,
}

impl MessagingAgUiWiring {
    pub(super) fn run(&self) -> chat_pipeline::AgUiRun<'_> {
        chat_pipeline::AgUiRun {
            run_id: self.scope.run_id().to_owned(),
            thread_id: Some(self.thread_id.clone()),
            sink: &self.sink,
        }
    }

    /// Render the final assistant reply through the status adapter
    /// (collapsing the placeholder into the final message).
    ///
    /// Returns `true` when the adapter actually sent the reply — the
    /// caller skips the standard `send_outbound_response` path to
    /// avoid posting the reply twice.
    pub(super) async fn finalize_reply(&self, reply: &str) -> bool {
        let Some(adapter) = self.status_adapter.as_ref() else {
            return false;
        };
        match adapter.finalize(reply).await {
            Ok(()) => true,
            Err(e) => {
                warn!(
                    run_id = %self.scope.run_id(),
                    error = %e,
                    "status adapter finalize failed; falling back to normal send"
                );
                false
            }
        }
    }
}

impl Drop for MessagingAgUiWiring {
    fn drop(&mut self) {
        // When the dispatch errors out before `finalize_reply` runs,
        // abort the consumer task explicitly so it doesn't linger on
        // a broadcast that may still be open (the scope drop unblocks
        // it via `Closed`, but aborting is faster and covers the case
        // where the registry retains a clone of the sender elsewhere).
        if let Some(handle) = self.status_consumer.take() {
            handle.abort();
        }
    }
}

/// Build AG-UI wiring for a messaging dispatch.
///
/// Returns `None` when the user id on the session is not a valid UUID.
///
/// In that case the messaging turn still runs but without progress
/// feedback — the channel link layer already rejects malformed user
/// ids, so this arm is effectively dead in practice.
///
/// When the channel supports in-place progress updates (Telegram,
/// Slack, Discord), this also opens a [`StatusAdapter`] and spawns
/// a background task that mirrors AG-UI events as `editMessageText`
/// / `chat.update` / `PATCH messages` calls.
///
/// `WhatsApp` and Messenger fall through to progress-free dispatch
/// because their APIs cannot edit previously-sent messages.
pub(super) async fn setup_messaging_agui(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
) -> Option<MessagingAgUiWiring> {
    // Use the authoritative user id from AuthResult — same source as every
    // other request-scoped principal in the codebase. session.user_id is
    // kept on the session row for audit/identity continuity across re-binds
    // but the dispatch's authenticated principal is AuthResult.
    let user_id = dispatch.auth_result.user_id;
    let run_id = Uuid::new_v4().to_string();
    let owner = RunOwner::new(user_id, dispatch.user_tenant_id);
    let scope = dispatch
        .resources
        .agui_registry
        .register_scoped(&run_id, owner);
    let sink = BroadcastSink::new(
        (*dispatch.resources.agui_registry).clone(),
        AgUiEventFilter::default(),
    );
    info!(
        run_id = %run_id,
        channel = %dispatch.channel,
        user_id = %user_id,
        "AG-UI run registered for messaging turn"
    );

    // Open the per-channel status adapter and spin up the consumer
    // task *after* the run is registered — otherwise the consumer's
    // `subscribe_self` would race the register call and miss early
    // events even with the replay backlog.
    let (status_adapter, status_consumer) =
        maybe_open_status_bridge(dispatch, channel_config, &run_id).await;

    Some(MessagingAgUiWiring {
        scope,
        sink,
        thread_id: dispatch.session.conversation.clone(),
        status_adapter,
        status_consumer,
    })
}

/// Open a status adapter + consumer task against the pre-loaded
/// channel config.
///
/// Returns `(None, None)` when either:
///
/// - (a) the channel does not support progress rendering (`WhatsApp`/Messenger);
/// - (b) the dispatch lacks a `conversation_id`;
/// - (c) the placeholder send fails.
///
/// Any of these are treated as "skip progress, deliver a single
/// reply at the end" rather than a hard failure. The caller passes
/// the `channel_config` snapshot already loaded by
/// `dispatch_and_respond` so the bridge stays out of the DB path.
async fn maybe_open_status_bridge(
    dispatch: &PendingDispatch,
    channel_config: &ChannelConfig,
    run_id: &str,
) -> (
    Option<Arc<dyn StatusAdapter + Send + Sync>>,
    Option<JoinHandle<()>>,
) {
    let conversation_id = match dispatch.conversation_id.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => return (None, None),
    };

    // Localized "thinking…" placeholder — resolved from the messaging-strings
    // registry for the user's locale so Telegram/Slack/Discord show the
    // matching-language progress message.
    let placeholder_text = dispatch
        .resources
        .messaging_strings_registry
        .get(KEY_THINKING_PLACEHOLDER, &dispatch.locale);

    let params = OpenStatusParams {
        channel_type: dispatch.channel_type,
        channel_config,
        conversation_id,
        thread_id: dispatch.thread_id.as_deref(),
        placeholder_text: &placeholder_text,
        // Production always hits the real platform base URLs; the
        // override exists so integration tests can point at a local
        // mock server to verify dispatch routing per `ChannelType`.
        api_base_override: None,
    };
    let Some(adapter) = open_status_adapter(&params).await else {
        return (None, None);
    };

    let consumer = spawn_status_consumer(
        &dispatch.resources.agui_registry,
        run_id.to_owned(),
        Arc::clone(&adapter),
        Arc::clone(&dispatch.resources.messaging_strings_registry),
        dispatch.locale.clone(),
    );
    (Some(adapter), consumer)
}
