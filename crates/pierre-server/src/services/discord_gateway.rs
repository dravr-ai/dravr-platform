// ABOUTME: Discord Gateway integration bridging real-time WebSocket events to the webhook pipeline
// ABOUTME: Spawns dravr-canot Gateway client and routes MESSAGE_CREATE events through persist+dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::str::FromStr;
use std::sync::Arc;

use pierre_core::models::messaging::{ChannelType, IncomingMessage};
use pierre_core::models::TenantId;
use pierre_database::repositories::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::channels::discord::gateway::{start_gateway, GatewayConfig};
use pierre_messaging::factory::create_adapter_from_config;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress;

/// Start the Discord Gateway background service
///
/// Reads `DISCORD_BOT_TOKEN` from environment. If present, spawns the Gateway
/// WebSocket client and a message processor that feeds incoming messages through
/// the same pipeline as webhook-delivered messages (persist → LLM dispatch → reply).
pub fn start_discord_gateway(resources: &Arc<ServerContext>) {
    let bot_token = match env::var("DISCORD_BOT_TOKEN") {
        Ok(t) if !t.is_empty() => t,
        _ => {
            info!("Discord Gateway: DISCORD_BOT_TOKEN not set, skipping");
            return;
        }
    };

    let config = GatewayConfig::new(bot_token);
    let (tx, rx) = mpsc::channel(256);

    tokio::spawn(async move {
        start_gateway(config, tx).await;
        warn!("Discord Gateway: client stopped");
    });

    let processor_resources = Arc::clone(resources);
    tokio::spawn(async move {
        process_gateway_messages(processor_resources, rx).await;
    });

    info!("Discord Gateway: started");
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

/// Load the Discord channel config from DB and construct an adapter
async fn resolve_channel_config(
    resources: &ServerContext,
) -> Option<(TenantId, Arc<dyn MessagingChannel>)> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let configs = db.get_configs_by_channel_type("discord").await.ok()?;
    let config = configs.first()?;
    let tenant_id = parse_tenant_id(config)?;
    let adapter = create_adapter_from_config(ChannelType::Discord, config).ok()?;
    Some((tenant_id, adapter))
}

/// Extract and parse `tenant_id` from a channel config JSON value
fn parse_tenant_id(config: &Value) -> Option<TenantId> {
    let tenant_id_str = config.get("tenant_id").and_then(|v| v.as_str())?;
    TenantId::from_str(tenant_id_str).ok()
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
    )
    .await;

    for dispatch in pending_dispatches {
        tokio::spawn(async move {
            messaging_ingress::dispatch_and_respond(dispatch).await;
        });
    }
}
