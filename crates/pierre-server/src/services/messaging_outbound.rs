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
use pierre_database::backends::MessagingRepository;
use pierre_messaging::channel::MessagingChannel;
use pierre_messaging::factory::create_adapter_from_config;
use pierre_messaging::retry::{compute_retry_update, RetryDecision};
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use serde_json::Value;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::mcp::resources::ServerContext;
use crate::services::analytics::{analytics, hash_id};

/// Polling interval for the outbound retry worker
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum entries to process per poll cycle
const BATCH_SIZE: i64 = 20;

/// Start the background outbound retry worker
///
/// Spawns a tokio task that polls the outbound queue every `POLL_INTERVAL` seconds.
/// For each pending entry, loads the channel config, constructs an adapter, and
/// attempts delivery. On failure, applies exponential backoff via `compute_retry_update`.
pub fn start_outbound_worker(resources: Arc<ServerContext>) {
    tokio::spawn(async move {
        info!("Messaging outbound retry worker started");
        loop {
            if let Err(e) = process_pending_batch(&resources).await {
                error!(error = %e, "Outbound retry worker batch failed");
            }
            sleep(POLL_INTERVAL).await;
        }
    });
}

/// Process one batch of pending outbound entries
async fn process_pending_batch(resources: &Arc<ServerContext>) -> Result<(), AppError> {
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
    /// User who originated the outbound message — read from the
    /// `messaging_outbound_queue.user_id` column. Used as the `PostHog`
    /// `distinct_id` for delivery + dead-letter analytics so funnels
    /// stay user-scoped instead of tenant-scoped.
    user_id: Option<&'a str>,
    payload_str: &'a str,
    attempt_count: i64,
    /// Conversation-turn correlation identifier persisted on the queue row.
    /// Threaded into the retry `send_raw` call so the resulting
    /// [`DeliveryReceipt`] keeps the same turn id as the original send.
    ///
    /// Malformed or missing values fall back to the nil UUID sentinel —
    /// the same one the `turn_id` DB column defaults to for rows that
    /// predate turn-id threading.
    turn_id: CanotTurnId,
}

/// Extract fields from a raw JSON outbound entry
fn parse_entry_fields(entry: &Value) -> EntryFields<'_> {
    let turn_uuid = entry["turn_id"]
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .unwrap_or_else(uuid::Uuid::nil);
    EntryFields {
        entry_id: entry["id"].as_str().unwrap_or_default(),
        channel_type_str: entry["channel_type"].as_str().unwrap_or_default(),
        tenant_id_str: entry["tenant_id"].as_str().unwrap_or_default(),
        user_id: entry["user_id"].as_str(),
        payload_str: entry["payload"].as_str().unwrap_or("{}"),
        attempt_count: entry["attempt_count"].as_i64().unwrap_or(0),
        turn_id: CanotTurnId::from_uuid(turn_uuid),
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

    let Some(prepared) = prepare_delivery(db, &fields, tenant_id, channel_type).await else {
        return;
    };

    attempt_delivery(db, &prepared.0, &prepared.1, &prepared.2, &fields).await;
}

/// Load config, create adapter, and parse payload for delivery
///
/// Returns `None` if any step fails (already logged).
async fn prepare_delivery(
    db: &dyn MessagingRepository,
    fields: &EntryFields<'_>,
    tenant_id: TenantId,
    channel_type: ChannelType,
) -> Option<(Arc<dyn MessagingChannel>, Value, ChannelConfig)> {
    let config = load_entry_config(
        db,
        fields.entry_id,
        tenant_id,
        fields.channel_type_str,
        fields.attempt_count,
    )
    .await?;

    let adapter = match create_adapter_from_config(channel_type, &config) {
        Ok(a) => a,
        Err(e) => {
            error!(error = %e, "Failed to create adapter for retry");
            return None;
        }
    };

    let payload: Value = serde_json::from_str(fields.payload_str).unwrap_or_default();
    let channel_config = match serde_json::from_value::<ChannelConfig>(config) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Failed to deserialize channel config for retry");
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
            error!(error = %e, "Failed to load channel config for retry");
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
    fields: &EntryFields<'_>,
) {
    let hashed_tenant = hash_id(fields.tenant_id_str);

    match adapter
        .send_raw(payload, fields.turn_id, channel_config)
        .await
    {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            info!(
                entry_id = %fields.entry_id,
                channel_message_id = %channel_msg_id,
                "Outbound retry delivery succeeded"
            );
            // Prefer hashed user_id as the PostHog distinct_id when the
            // queue row carries one; fall back to tenant hash for rows
            // enqueued before user_id became required.
            let distinct_id = fields
                .user_id
                .map_or_else(|| hashed_tenant.clone(), hash_id);
            analytics().track_outbound_delivered(
                fields.channel_type_str,
                &distinct_id,
                fields.attempt_count > 0,
            );
            let _ = db
                .update_outbound_status(
                    fields.entry_id,
                    "sent",
                    i32::try_from(fields.attempt_count + 1).unwrap_or(i32::MAX),
                    None,
                )
                .await;
        }
        Err(e) => {
            warn!(
                error = %e,
                entry_id = %fields.entry_id,
                attempt = fields.attempt_count + 1,
                "Outbound delivery failed"
            );
            handle_retry_decision(
                db,
                fields.entry_id,
                fields.attempt_count,
                fields.user_id,
                fields.channel_type_str,
            )
            .await;
        }
    }
}

/// Apply retry backoff or dead-letter based on attempt count.
///
/// `user_id` is the originating user from the queue row; when present it
/// becomes the (hashed) `PostHog` `distinct_id` on the dead-letter analytics
/// event. Rows that predate the `user_id` column fall back to the
/// `entry_id` so `PostHog` still receives a stable identifier.
async fn handle_retry_decision(
    db: &dyn MessagingRepository,
    entry_id: &str,
    attempt_count: i64,
    user_id: Option<&str>,
    channel_type: &str,
) {
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
            let distinct_id = user_id.map_or_else(|| entry_id.to_owned(), hash_id);
            analytics().track_error(channel_type, &distinct_id, "dead_lettered");
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
