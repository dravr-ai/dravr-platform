// ABOUTME: Background outbound retry worker for messaging queue
// ABOUTME: Polls pending outbound entries, retries delivery with exponential backoff, dead-letters failures
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use pierre_core::errors::AppError;
use pierre_core::models::messaging::{ChannelConfig, ChannelType};
use pierre_core::models::TenantId;
use pierre_database::plugins::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::retry::{compute_retry_update, RetryDecision};
use serde_json::Value;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use crate::mcp::resources::ServerResources;

/// Polling interval for the outbound retry worker
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum entries to process per poll cycle
const BATCH_SIZE: i64 = 20;

/// Start the background outbound retry worker
///
/// Spawns a tokio task that polls the outbound queue every `POLL_INTERVAL` seconds.
/// For each pending entry, loads the channel config, constructs an adapter, and
/// attempts delivery. On failure, applies exponential backoff via `compute_retry_update`.
pub fn start_outbound_worker(resources: Arc<ServerResources>) {
    tokio::spawn(async move {
        info!("Messaging outbound retry worker started");
        loop {
            if let Err(e) = process_pending_batch(&resources).await {
                warn!(error = %e, "Outbound retry worker batch failed");
            }
            sleep(POLL_INTERVAL).await;
        }
    });
}

/// Process one batch of pending outbound entries
async fn process_pending_batch(resources: &Arc<ServerResources>) -> Result<(), AppError> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let entries = db.get_all_pending_outbound(BATCH_SIZE).await?;

    if entries.is_empty() {
        return Ok(());
    }

    debug!(count = entries.len(), "Processing outbound retry batch");

    for entry in &entries {
        process_single_entry(db, entry).await;
    }

    Ok(())
}

/// Parsed fields from an outbound queue entry
struct EntryFields<'a> {
    entry_id: &'a str,
    channel_type_str: &'a str,
    tenant_id_str: &'a str,
    payload_str: &'a str,
    attempt_count: i64,
}

/// Extract fields from a raw JSON outbound entry
fn parse_entry_fields(entry: &Value) -> EntryFields<'_> {
    EntryFields {
        entry_id: entry["id"].as_str().unwrap_or_default(),
        channel_type_str: entry["channel_type"].as_str().unwrap_or_default(),
        tenant_id_str: entry["tenant_id"].as_str().unwrap_or_default(),
        payload_str: entry["payload"].as_str().unwrap_or("{}"),
        attempt_count: entry["attempt_count"].as_i64().unwrap_or(0),
    }
}

/// Process a single outbound queue entry: load config, construct adapter, attempt delivery
async fn process_single_entry(db: &dyn MessagingRepository, entry: &Value) {
    let fields = parse_entry_fields(entry);

    let Ok(channel_type) = ChannelType::from_str(fields.channel_type_str) else {
        warn!(
            entry_id = %fields.entry_id,
            channel_type = %fields.channel_type_str,
            "Unknown channel type in outbound queue, dead-lettering"
        );
        dead_letter(db, fields.entry_id, fields.attempt_count).await;
        return;
    };

    let Ok(tenant_id) = TenantId::from_str(fields.tenant_id_str) else {
        warn!(
            entry_id = %fields.entry_id,
            tenant_id = %fields.tenant_id_str,
            "Invalid tenant_id in outbound queue entry, dead-lettering"
        );
        dead_letter(db, fields.entry_id, fields.attempt_count).await;
        return;
    };

    let Some(prepared) = prepare_delivery(
        db,
        fields.entry_id,
        tenant_id,
        fields.channel_type_str,
        channel_type,
        fields.payload_str,
        fields.attempt_count,
    )
    .await
    else {
        return;
    };

    attempt_delivery(
        db,
        &prepared.0,
        &prepared.1,
        &prepared.2,
        fields.entry_id,
        fields.attempt_count,
    )
    .await;
}

/// Load config, create adapter, and parse payload for delivery
///
/// Returns `None` if any step fails (already logged).
async fn prepare_delivery(
    db: &dyn MessagingRepository,
    entry_id: &str,
    tenant_id: TenantId,
    channel_type_str: &str,
    channel_type: ChannelType,
    payload_str: &str,
    attempt_count: i64,
) -> Option<(Arc<dyn MessagingChannel>, Value, ChannelConfig)> {
    let config =
        load_entry_config(db, entry_id, tenant_id, channel_type_str, attempt_count).await?;

    let adapter = match create_adapter_from_config(channel_type, &config) {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "Failed to create adapter for retry");
            return None;
        }
    };

    let payload: Value = serde_json::from_str(payload_str).unwrap_or_default();
    let channel_config = match serde_json::from_value::<ChannelConfig>(config) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to deserialize channel config for retry");
            return None;
        }
    };

    Some((adapter, payload, channel_config))
}

/// Load the channel config for an outbound entry, dead-lettering if not found
async fn load_entry_config(
    db: &dyn MessagingRepository,
    entry_id: &str,
    tenant_id: TenantId,
    channel_type_str: &str,
    attempt_count: i64,
) -> Option<Value> {
    match db.get_channel_config(tenant_id, channel_type_str).await {
        Ok(Some(cfg)) => Some(cfg),
        Ok(None) => {
            warn!(
                entry_id = %entry_id,
                channel = %channel_type_str,
                "No channel config found, dead-lettering"
            );
            dead_letter(db, entry_id, attempt_count).await;
            None
        }
        Err(e) => {
            warn!(error = %e, "Failed to load channel config for retry");
            None
        }
    }
}

/// Attempt delivery via the channel adapter, handling success and retry on failure
async fn attempt_delivery(
    db: &dyn MessagingRepository,
    adapter: &Arc<dyn MessagingChannel>,
    payload: &Value,
    channel_config: &ChannelConfig,
    entry_id: &str,
    attempt_count: i64,
) {
    match adapter.send_raw(payload, channel_config).await {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            info!(
                entry_id = %entry_id,
                channel_message_id = %channel_msg_id,
                "Outbound retry delivery succeeded"
            );
            let _ = db
                .update_outbound_status(
                    entry_id,
                    "sent",
                    i32::try_from(attempt_count + 1).unwrap_or(i32::MAX),
                    None,
                )
                .await;
        }
        Err(e) => {
            warn!(
                error = %e,
                entry_id = %entry_id,
                attempt = attempt_count + 1,
                "Outbound delivery failed"
            );
            handle_retry_decision(db, entry_id, attempt_count).await;
        }
    }
}

/// Apply retry backoff or dead-letter based on attempt count
async fn handle_retry_decision(db: &dyn MessagingRepository, entry_id: &str, attempt_count: i64) {
    let update = compute_retry_update(i32::try_from(attempt_count).unwrap_or(i32::MAX));
    match update.decision {
        RetryDecision::Retry {
            next_retry_at,
            ref status,
        } => {
            let retry_at = next_retry_at.to_rfc3339();
            let _ = db
                .update_outbound_status(entry_id, status, update.attempt_count, Some(&retry_at))
                .await;
        }
        RetryDecision::DeadLetter => {
            warn!(
                entry_id = %entry_id,
                "All retries exhausted, moving to dead-letter queue"
            );
            let _ = db
                .update_outbound_status(entry_id, "dlq", update.attempt_count, None)
                .await;
        }
    }
}

/// Move an entry to the dead-letter queue
async fn dead_letter(db: &dyn MessagingRepository, entry_id: &str, attempt_count: i64) {
    let _ = db
        .update_outbound_status(
            entry_id,
            "dlq",
            i32::try_from(attempt_count + 1).unwrap_or(i32::MAX),
            None,
        )
        .await;
}
