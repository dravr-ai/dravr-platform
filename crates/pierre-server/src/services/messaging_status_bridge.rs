// ABOUTME: AG-UI → channel progress bridge — opens per-channel StatusAdapter and drives set_status
// ABOUTME: Subscribes to the messaging run's broadcast, deserializes each event, forwards to adapter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Bridge between the server-side AG-UI [`RunRegistry`] and the
//! client-side [`pierre_messaging::agui_status::StatusAdapter`]s.
//!
//! Messaging channels (Telegram/Slack/Discord) let a user see what
//! the assistant is doing by editing a single placeholder message
//! in place as the pipeline advances.
//!
//! Each channel's adapter owns the `editMessageText` /
//! `chat.update` / `PATCH messages` call; the bridge owns the glue
//! that turns AG-UI events streamed through the registry into
//! adapter calls.
//!
//! Flow per messaging turn:
//!
//! 1. `messaging_ingress::dispatch_and_respond` calls
//!    [`open_status_adapter`] with the inbound dispatch. The bridge
//!    loads the per-tenant channel credentials and sends a
//!    placeholder "…" message.
//! 2. Once the AG-UI run is registered, the caller hands the run id
//!    and the adapter to [`spawn_status_consumer`] which subscribes
//!    to the registry and drives progress edits in a background task.
//! 3. When the pipeline finishes, the caller calls
//!    [`StatusAdapter::finalize`] with the assistant reply so the
//!    placeholder collapses into the final message. The background
//!    consumer observes the `RUN_FINISHED` event and exits cleanly
//!    on its own (the broadcast channel closes when the `RunScope`
//!    drops).
//!
//! `WhatsApp` and Messenger have no edit-message semantic (each
//! `sendMessage` is a new row in the user's chat history).
//!
//! Bridging status through them would spam the conversation, so
//! those channels return `None` and fall back to the normal
//! single-reply path.

use pierre_messaging::agui_consumer::AgUiEvent;
use pierre_messaging::agui_status::{drive_status_updates, StatusAdapter};
use pierre_messaging::channels::discord::agui_status::DiscordStatusAdapter;
use pierre_messaging::channels::slack::agui_status::SlackStatusAdapter;
use pierre_messaging::channels::telegram::agui_status::TelegramStatusAdapter;
use pierre_messaging::models::{ChannelConfig, ChannelType};
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::agui::RunRegistry;

/// Channel-neutral placeholder text used by every adapter. Kept here
/// (not per-channel) so the UX is uniform across Telegram/Slack/Discord.
const PLACEHOLDER_TEXT: &str = "thinking…";

/// Parameters for opening a channel-specific [`StatusAdapter`].
///
/// Carries the per-tenant `ChannelConfig` (holds the bot token /
/// bearer token) plus the channel-local conversation identifier.
///
/// Kept struct-shaped so callers can grow it without a new
/// positional-argument explosion.
pub struct OpenStatusParams<'a> {
    /// Inbound message's channel platform — drives adapter selection.
    pub channel_type: ChannelType,
    /// Per-tenant `ChannelConfig`.
    ///
    /// Holds the bot/bearer token and other credentials the channel's
    /// outbound API requires.
    pub channel_config: &'a ChannelConfig,
    /// Channel-local conversation identifier.
    ///
    /// Telegram chat id / Slack channel id / Discord channel id.
    /// Passed straight through to the adapter's `open` call.
    pub conversation_id: &'a str,
    /// Optional Slack `thread_ts` / Telegram `message_thread_id`.
    ///
    /// When the inbound message arrived inside a thread, the
    /// placeholder should land there too — not as a top-level
    /// channel message.
    pub thread_id: Option<&'a str>,
}

/// Open the per-channel [`StatusAdapter`] for `params`, sending the
/// initial placeholder message.
///
/// Returns `None` when the channel does not support in-place progress
/// updates (`WhatsApp`, Messenger) or when the required credentials
/// are missing from `channel_config`. The caller MUST treat `None` as
/// "skip progress rendering, send a single final reply instead"
/// rather than tearing the turn down.
pub async fn open_status_adapter(
    params: &OpenStatusParams<'_>,
) -> Option<Arc<dyn StatusAdapter + Send + Sync>> {
    match params.channel_type {
        ChannelType::Telegram => open_telegram(params).await,
        ChannelType::Slack => open_slack(params).await,
        ChannelType::Discord => open_discord(params).await,
        // WhatsApp Cloud API and Messenger Send API both lack an
        // "edit a previous message" endpoint — issuing status via
        // new sendMessage calls would spam the thread. Intentionally
        // return None; the messaging turn falls back to the single
        // final-reply path.
        ChannelType::WhatsApp | ChannelType::Messenger => None,
    }
}

async fn open_telegram(
    params: &OpenStatusParams<'_>,
) -> Option<Arc<dyn StatusAdapter + Send + Sync>> {
    let bot_token = params.channel_config.bot_token.as_deref()?;
    let thread_id = params.thread_id.and_then(|t| t.parse::<i64>().ok());
    match TelegramStatusAdapter::open(
        bot_token,
        params.conversation_id,
        thread_id,
        PLACEHOLDER_TEXT,
    )
    .await
    {
        Ok(adapter) => Some(Arc::new(adapter) as Arc<dyn StatusAdapter + Send + Sync>),
        Err(e) => {
            warn!(error = %e, "Telegram status placeholder open failed; skipping progress rendering");
            None
        }
    }
}

async fn open_slack(params: &OpenStatusParams<'_>) -> Option<Arc<dyn StatusAdapter + Send + Sync>> {
    // Slack's bot credential lives in `api_key` (format `xoxb-...`);
    // `bot_token` is used by channels whose API uses a distinct prefix
    // (Discord, for example).
    let bot_token = params.channel_config.api_key.as_deref()?;
    let thread_ts = params.thread_id.map(str::to_owned);
    match SlackStatusAdapter::open(
        bot_token,
        params.conversation_id,
        thread_ts,
        PLACEHOLDER_TEXT,
    )
    .await
    {
        Ok(adapter) => Some(Arc::new(adapter) as Arc<dyn StatusAdapter + Send + Sync>),
        Err(e) => {
            warn!(error = %e, "Slack status placeholder open failed; skipping progress rendering");
            None
        }
    }
}

async fn open_discord(
    params: &OpenStatusParams<'_>,
) -> Option<Arc<dyn StatusAdapter + Send + Sync>> {
    let bot_token = params.channel_config.bot_token.as_deref()?;
    match DiscordStatusAdapter::open(bot_token, params.conversation_id, PLACEHOLDER_TEXT).await {
        Ok(adapter) => Some(Arc::new(adapter) as Arc<dyn StatusAdapter + Send + Sync>),
        Err(e) => {
            warn!(error = %e, "Discord status placeholder open failed; skipping progress rendering");
            None
        }
    }
}

/// Spawn a background task that forwards AG-UI events from `run_id`
/// into `adapter` until the pipeline finishes.
///
/// Drains the replay backlog first (so adapters that attach after
/// the first few events still render `RUN_STARTED` → `STEP_STARTED`)
/// then switches to the live receiver.
///
/// Exits on `RUN_FINISHED`, `RUN_ERROR`, or broadcast close (which
/// the `RunScope` triggers when the dispatch task drops it).
///
/// The returned `JoinHandle` is abort-safe — the caller can drop it
/// to let the task finish naturally, or `.abort()` it to stop early
/// (for example when `dispatch_and_respond` fails before pipeline
/// entry and the adapter needs no further edits).
pub fn spawn_status_consumer(
    registry: &Arc<RunRegistry>,
    run_id: String,
    adapter: Arc<dyn StatusAdapter + Send + Sync>,
) -> Option<JoinHandle<()>> {
    let subscription = registry.subscribe_self(&run_id)?;
    Some(tokio::spawn(async move {
        run_status_loop(run_id, adapter, subscription.backlog, subscription.receiver).await;
    }))
}

async fn run_status_loop(
    run_id: String,
    adapter: Arc<dyn StatusAdapter + Send + Sync>,
    backlog: Vec<String>,
    mut receiver: broadcast::Receiver<String>,
) {
    // Replay any events already in the buffer first so the user's
    // placeholder transitions through the stages that landed between
    // `register_scoped` and the consumer's subscribe.
    for raw in backlog {
        if forward_event(&run_id, &adapter, &raw).await {
            return;
        }
    }
    loop {
        match receiver.recv().await {
            Ok(raw) => {
                if forward_event(&run_id, &adapter, &raw).await {
                    return;
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                debug!(
                    run_id = %run_id,
                    "AG-UI broadcast closed; status consumer exiting"
                );
                return;
            }
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                // The pipeline emits events faster than the adapter
                // can edit messages (Telegram's ~1 edit/s is the
                // bottleneck). Lagging is expected under load —
                // dropping the oldest N keeps the user looking at
                // the *most recent* transition, which is the right
                // UX choice.
                debug!(
                    run_id = %run_id,
                    skipped,
                    "AG-UI broadcast lagged for status consumer"
                );
            }
        }
    }
}

/// Deserialize `raw` to an [`AgUiEvent`] and forward it to `adapter`.
///
/// Returns `true` when the consumer loop should exit (terminal event
/// observed).
///
/// Malformed events are logged and skipped — a parse error on one
/// event must not tear down the whole stream because future event
/// kinds deserialize into `AgUiEvent::Unknown` under canot's
/// `#[serde(other)]` guard rather than bubbling here.
async fn forward_event(
    run_id: &str,
    adapter: &Arc<dyn StatusAdapter + Send + Sync>,
    raw: &str,
) -> bool {
    let event: AgUiEvent = match serde_json::from_str(raw) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                run_id = %run_id,
                error = %e,
                "AG-UI event JSON parse failed in status bridge; skipping"
            );
            return false;
        }
    };
    let terminal = matches!(
        event,
        AgUiEvent::RunFinished { .. } | AgUiEvent::RunError { .. }
    );
    if let Err(e) = drive_status_updates(adapter.as_ref(), &event).await {
        // Adapter errors are non-fatal — a single failed edit
        // shouldn't tear down the whole consumer. Log and keep going
        // (the next event will likely succeed; if the channel is
        // fundamentally broken the `finalize` call will surface it).
        debug!(
            run_id = %run_id,
            error = %e,
            "status adapter set_status failed; continuing"
        );
    }
    terminal
}
