// ABOUTME: Discord Gateway integration bridging real-time WebSocket events to the webhook pipeline
// ABOUTME: Spawns dravr-canot Gateway client; MESSAGE_CREATE runs persist+dispatch, reactions become message feedback
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::sync::Arc;

use dravr_canot::channel::MessagingChannel;
use dravr_canot::channels::discord::gateway::{start_gateway, GatewayConfig, GatewaySinks};
use pierre_core::models::messaging::{ChannelType, InboundReaction, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;
use crate::services::messaging_ingress::reactions;

/// Start the Discord Gateway background service
///
/// Reads `DISCORD_BOT_TOKEN` from environment. If present, spawns the Gateway
/// WebSocket client and a message processor that feeds incoming messages through
/// the same pipeline as webhook-delivered messages (persist → LLM dispatch → reply).
/// Emoji reactions ride their own channel to the shared reaction consumer: they
/// rate a reply already sent, so they never open a turn.
pub fn start_discord_gateway(resources: &Arc<ServerContext>) {
    let bot_token = match env::var("DISCORD_BOT_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            info!("Discord Gateway: DISCORD_BOT_TOKEN not set, skipping");
            return;
        }
    };

    let config = gateway_config(
        bot_token,
        resources.mcp.channel_adapters.discord_gateway_url(),
    );
    let (tx, rx) = mpsc::channel::<IncomingMessage>(256);
    let (reaction_tx, reaction_rx) = mpsc::channel::<InboundReaction>(256);
    let sinks = GatewaySinks {
        messages: tx,
        reactions: reaction_tx,
    };

    tokio::spawn(async move {
        start_gateway(config, sinks).await;
        warn!("Discord Gateway: client stopped");
    });

    let processor_resources = Arc::clone(resources);
    tokio::spawn(async move {
        process_gateway_messages(processor_resources, rx).await;
    });

    let reaction_resources = Arc::clone(resources);
    tokio::spawn(async move {
        reactions::consume_reactions(reaction_resources, "Discord Gateway", reaction_rx).await;
    });

    info!("Discord Gateway: started");
}

/// The Gateway client's configuration for `bot_token`: Discord's own
/// Gateway, or `gateway_url` when the context's adapter factory names one
/// ([`ChannelAdapterFactory::discord_gateway_url`]), which only a test
/// context's does.
///
/// [`ChannelAdapterFactory::discord_gateway_url`]: pierre_services::channel_adapters::ChannelAdapterFactory::discord_gateway_url
fn gateway_config(bot_token: String, gateway_url: Option<String>) -> GatewayConfig {
    let config = GatewayConfig::new(bot_token);
    match gateway_url {
        Some(url) => config.with_gateway_url(url),
        None => config,
    }
}

/// Process incoming messages from the Gateway mpsc channel
async fn process_gateway_messages(
    resources: Arc<ServerContext>,
    mut rx: mpsc::Receiver<IncomingMessage>,
) {
    while let Some(message) = rx.recv().await {
        info!(
            sender_id = %message.sender_id,
            sender_name = ?message.sender_name,
            "Discord Gateway: received message"
        );
        let Some((tenant_id, adapter)) = resolve_channel_config(&resources).await else {
            continue;
        };
        dispatch_message(&resources, tenant_id, adapter, message).await;
    }
    info!("Discord Gateway: message processor stopped");
}

/// Load the Discord channel config from DB and construct an adapter through
/// the factory the context was built on, as the webhook routes do.
async fn resolve_channel_config(
    resources: &ServerContext,
) -> Option<(TenantId, Arc<dyn MessagingChannel>)> {
    let data = resources.data();
    let db: &dyn MessagingRepository = data.repos().messaging.as_ref();
    let configs = db.get_configs_by_channel_type("discord").await.ok()?;
    let config = configs.first()?;
    let tenant_id = parse_tenant_id(config)?;
    let adapter = resources
        .mcp
        .channel_adapters
        .build(ChannelType::Discord, config)
        .ok()?;
    Some((tenant_id, adapter))
}

/// Extract and parse `tenant_id` from a channel config JSON value
fn parse_tenant_id(config: &Value) -> Option<TenantId> {
    let tenant_id_str = config.get("tenant_id").and_then(|v| v.as_str())?;
    TenantId::parse_str(tenant_id_str).ok()
}

/// Persist the message and spawn LLM dispatch tasks
async fn dispatch_message(
    resources: &Arc<ServerContext>,
    tenant_id: TenantId,
    adapter: Arc<dyn MessagingChannel>,
    message: IncomingMessage,
) {
    let messages = vec![message];

    let (_stored, pending_dispatches) = messaging_ingress::persist_inbound(
        resources,
        "discord",
        tenant_id,
        ChannelType::Discord,
        &adapter,
        &messages,
        // Socket-mode turns open their placeholder where the context's
        // adapter factory says, as webhook turns do: the channel's real API
        // in a deployment.
        resources.mcp.channel_adapters.status_api_base().as_deref(),
    )
    .await;

    // Recorded and handed to the runner exactly like webhook turns: Discord Gateway
    // turns are detached from the event that started them and vanish the
    // same way on a scaledown, and the runner is what makes them survive it
    // (registre#126).
    for dispatch in pending_dispatches {
        messaging_ingress::start_turn(resources, dispatch).await;
    }
}
