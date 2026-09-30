// ABOUTME: Background outbound retry worker for messaging queue
// ABOUTME: Polls pending outbound entries, retries delivery with exponential backoff, dead-letters failures
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The outbound retry worker.
//!
//! A channel send that failed is queued (`messaging_outbound_queue`) with its
//! rendered payload, and this worker re-sends it on the backoff schedule
//! `dravr_canot::retry` defines, dead-lettering it once the attempts run
//! out. The queue is durable, so a restart resumes where it stopped.
//!
//! Two optional bounds on an entry decide whether it may still go out at all,
//! and are checked before every attempt:
//!
//! - `expires_at`: the payload carries a link that expires; past this instant
//!   the entry is given up rather than sent with a dead link.
//! - `reauth_tenant_id` + `reauth_provider` (with the entry's `user_id`): the
//!   entry is a reconnect notice for that provider connection, sent only while
//!   the connection is still `needs_reauth`. Once the athlete reconnects (or
//!   the connection is revoked or removed) the entry is cancelled.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dravr_canot::channel::MessagingChannel;
use dravr_canot::retry::{compute_retry_update, RetryDecision};
use dravr_canot::turn::ConversationTurnId as CanotTurnId;
use pierre_core::errors::AppError;
use pierre_core::models::messaging::{ChannelConfig, ChannelType};
use pierre_core::models::{ConnectionStatus, TenantId};
use pierre_database::backends::{MessagingRepository, ProviderConnectionRepository};
use serde_json::Value;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::channel_adapters::ChannelAdapterFactory;

/// Polling interval for the outbound retry worker
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Maximum entries to process per poll cycle
const BATCH_SIZE: i64 = 20;

/// Queue status of an entry that will not be sent because what it said no
/// longer applies (a reconnect notice whose connection was reconnected).
/// Outside the worker's `pending`/`retrying:N` poll, so it is never picked up
/// again.
const STATUS_CANCELLED: &str = "cancelled";

/// Queue status of an entry the worker gave up on.
const STATUS_DEAD_LETTER: &str = "dlq";

/// The repositories and adapter factory one retry pass works against.
#[derive(Clone, Copy)]
pub struct OutboundRetryContext<'a> {
    /// The outbound queue and the channel configs.
    pub messaging: &'a dyn MessagingRepository,
    /// Connection state, for entries guarded by a reconnect.
    pub connections: &'a dyn ProviderConnectionRepository,
    /// Builds the adapter an entry is re-sent through.
    pub adapters: &'a dyn ChannelAdapterFactory,
}

/// Start the background outbound retry worker
///
/// Spawns a tokio task that polls the outbound queue every `POLL_INTERVAL` seconds
/// and runs [`process_pending_batch`] over it.
pub fn start_outbound_worker(
    messaging: Arc<dyn MessagingRepository>,
    connections: Arc<dyn ProviderConnectionRepository>,
    adapters: Arc<dyn ChannelAdapterFactory>,
) {
    tokio::spawn(async move {
        info!("Messaging outbound retry worker started");
        loop {
            let ctx = OutboundRetryContext {
                messaging: messaging.as_ref(),
                connections: connections.as_ref(),
                adapters: adapters.as_ref(),
            };
            if let Err(e) = process_pending_batch(ctx).await {
                error!(error = %e, "Outbound retry worker batch failed");
            }
            sleep(POLL_INTERVAL).await;
        }
    });
}

/// Process one batch of due outbound entries.
///
/// For each entry: give it up when its `expires_at` has passed, cancel it when
/// it is a reconnect notice whose connection is no longer `needs_reauth`,
/// otherwise load the channel config, build the adapter and attempt delivery,
/// applying exponential backoff via `compute_retry_update` on failure.
///
/// # Errors
///
/// Returns the repository error when the due entries cannot be read.
pub async fn process_pending_batch(ctx: OutboundRetryContext<'_>) -> Result<(), AppError> {
    let entries = ctx.messaging.get_all_pending_outbound(BATCH_SIZE).await?;

    if entries.is_empty() {
        return Ok(());
    }

    debug!(count = entries.len(), "Processing outbound retry batch");

    for entry in &entries {
        process_single_entry(ctx, entry).await;
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
    /// Conversation-turn correlation identifier persisted on the queue row.
    /// Threaded into the retry `send_raw` call so the resulting
    /// [`DeliveryReceipt`] keeps the same turn id as the original send.
    ///
    /// Malformed or missing values fall back to the nil UUID sentinel —
    /// the same one the `turn_id` DB column defaults to for rows that
    /// predate turn-id threading.
    turn_id: CanotTurnId,
    /// Last instant the payload may be sent, when it carries an expiring link.
    expires_at: Option<DateTime<Utc>>,
    /// The connection a reconnect notice is about, when the entry is one.
    reauth: ReauthGuard<'a>,
}

/// What an entry says about the provider connection it is a reconnect
/// notice for.
enum ReauthGuard<'a> {
    /// Not a reconnect notice.
    Unguarded,
    /// A reconnect notice for this connection.
    Connection {
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &'a str,
    },
    /// Names a provider, but its user or tenant does not parse: the
    /// connection cannot be checked, so the notice is never sent unguarded.
    Unresolvable,
}

/// Whether an entry may still be sent.
enum Sendability {
    /// Send it.
    Due,
    /// The link it carries is about to expire: give it up.
    Expired,
    /// A reconnect notice for a connection no longer `needs_reauth`.
    Moot,
    /// The connection state could not be read; leave the entry for the next
    /// poll without spending an attempt.
    Unknown,
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
        payload_str: entry["payload"].as_str().unwrap_or("{}"),
        attempt_count: entry["attempt_count"].as_i64().unwrap_or(0),
        turn_id: CanotTurnId::from_uuid(turn_uuid),
        expires_at: entry["expires_at"]
            .as_str()
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|at| at.with_timezone(&Utc)),
        reauth: parse_reauth_target(entry),
    }
}

/// The reconnect guard an entry carries.
fn parse_reauth_target(entry: &Value) -> ReauthGuard<'_> {
    let Some(provider) = entry["reauth_provider"].as_str() else {
        return ReauthGuard::Unguarded;
    };
    let user_id = entry["user_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok());
    let tenant_id = entry["reauth_tenant_id"]
        .as_str()
        .and_then(|raw| TenantId::parse_str(raw).ok());
    match (user_id, tenant_id) {
        (Some(user_id), Some(tenant_id)) => ReauthGuard::Connection {
            user_id,
            tenant_id,
            provider,
        },
        _ => ReauthGuard::Unresolvable,
    }
}

/// Decide whether an entry may still be sent: before its link expires, and,
/// for a reconnect notice, only while its connection is still `needs_reauth`.
async fn sendability(
    connections: &dyn ProviderConnectionRepository,
    fields: &EntryFields<'_>,
) -> Sendability {
    if fields.expires_at.is_some_and(|at| at <= Utc::now()) {
        return Sendability::Expired;
    }
    let (user_id, tenant_id, provider) = match fields.reauth {
        ReauthGuard::Unguarded => return Sendability::Due,
        ReauthGuard::Unresolvable => return Sendability::Moot,
        ReauthGuard::Connection {
            user_id,
            tenant_id,
            provider,
        } => (user_id, tenant_id, provider),
    };
    match connections.get_for_user(user_id, Some(tenant_id)).await {
        Ok(found) => {
            let still_flagged = found.iter().any(|connection| {
                connection.provider == provider
                    && connection.status == ConnectionStatus::NeedsReauth
            });
            if still_flagged {
                Sendability::Due
            } else {
                Sendability::Moot
            }
        }
        Err(e) => {
            warn!(
                entry_id = %fields.entry_id,
                error = %e,
                "Outbound retry: connection state unreadable; entry left for the next poll"
            );
            Sendability::Unknown
        }
    }
}

/// Check an entry may still go out, settling the ones that may not: an entry
/// whose link expires is given up, a reconnect notice whose connection is no
/// longer flagged is cancelled, and one whose connection state cannot be read
/// is left for the next poll. Returns whether to attempt delivery.
async fn still_sendable(ctx: OutboundRetryContext<'_>, fields: &EntryFields<'_>) -> bool {
    let db = ctx.messaging;
    match sendability(ctx.connections, fields).await {
        Sendability::Due => true,
        Sendability::Unknown => false,
        Sendability::Expired => {
            warn!(
                entry_id = %fields.entry_id,
                channel = %fields.channel_type_str,
                attempts = fields.attempt_count,
                "Outbound retry: the link in the entry expires before it could be delivered; giving up"
            );
            dead_letter(db, fields.entry_id, fields.attempt_count).await;
            false
        }
        Sendability::Moot => {
            info!(
                entry_id = %fields.entry_id,
                channel = %fields.channel_type_str,
                "Outbound retry: the connection a reconnect notice was about is no longer flagged; cancelled"
            );
            let _ = db
                .update_outbound_status(
                    fields.entry_id,
                    STATUS_CANCELLED,
                    i32::try_from(fields.attempt_count).unwrap_or(i32::MAX),
                    None,
                )
                .await;
            false
        }
    }
}

/// Process a single outbound queue entry: check it may still go out, load
/// config, construct adapter, attempt delivery
async fn process_single_entry(ctx: OutboundRetryContext<'_>, entry: &Value) {
    let db = ctx.messaging;
    let fields = parse_entry_fields(entry);

    if !still_sendable(ctx, &fields).await {
        return;
    }

    let Ok(channel_type) = ChannelType::from_str(fields.channel_type_str) else {
        warn!(
            entry_id = %fields.entry_id,
            channel_type = %fields.channel_type_str,
            "Unknown channel type in outbound queue, dead-lettering"
        );
        dead_letter(db, fields.entry_id, fields.attempt_count).await;
        return;
    };

    let Ok(tenant_id) = TenantId::parse_str(fields.tenant_id_str) else {
        warn!(
            entry_id = %fields.entry_id,
            tenant_id = %fields.tenant_id_str,
            "Invalid tenant_id in outbound queue entry, dead-lettering"
        );
        dead_letter(db, fields.entry_id, fields.attempt_count).await;
        return;
    };

    let Some(prepared) = prepare_delivery(ctx, &fields, tenant_id, channel_type).await else {
        return;
    };

    attempt_delivery(db, &prepared.0, &prepared.1, &prepared.2, &fields).await;
}

/// Load config, create adapter, and parse payload for delivery
///
/// Returns `None` if any step fails (already logged).
async fn prepare_delivery(
    ctx: OutboundRetryContext<'_>,
    fields: &EntryFields<'_>,
    tenant_id: TenantId,
    channel_type: ChannelType,
) -> Option<(Arc<dyn MessagingChannel>, Value, ChannelConfig)> {
    let config = load_entry_config(
        ctx.messaging,
        fields.entry_id,
        tenant_id,
        fields.channel_type_str,
        fields.attempt_count,
    )
    .await?;

    let Some(adapter) = ctx.adapters.build(channel_type, &config) else {
        error!(
            entry_id = %fields.entry_id,
            channel = %fields.channel_type_str,
            "Failed to create adapter for retry"
        );
        return None;
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
    match adapter
        .send_raw(payload, fields.turn_id, channel_config)
        .await
    {
        Ok(receipt) => {
            let channel_msg_id = receipt.channel_message_id.as_deref().unwrap_or("");
            record_successful_delivery(db, fields, channel_msg_id).await;
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
                fields.tenant_id_str,
                fields.channel_type_str,
            )
            .await;
        }
    }
}

/// Log the successful retry delivery, emit the `outbound_delivered` notify
/// event, and mark the queue row `sent`.
async fn record_successful_delivery(
    db: &dyn MessagingRepository,
    fields: &EntryFields<'_>,
    channel_msg_id: &str,
) {
    info!(
        entry_id = %fields.entry_id,
        channel_message_id = %channel_msg_id,
        "Outbound retry delivery succeeded"
    );
    // Operational tier: the sink keys on the hashed tenant and drops the user
    // dimension, so emit `tenant_id` inline and omit user.
    info!(
        target: "notify",
        event = "messaging.outbound_delivered",
        tenant_id = %fields.tenant_id_str,
        channel = %fields.channel_type_str,
        is_retry = fields.attempt_count > 0,
        "outbound message delivered"
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

/// Apply retry backoff or dead-letter based on attempt count.
///
/// `tenant_id` is the raw tenant id from the queue row; on dead-letter it is
/// emitted inline on the operational `messaging.error` notify event, which the
/// analytics sink hashes into the tenant-scoped `distinct_id`.
async fn handle_retry_decision(
    db: &dyn MessagingRepository,
    entry_id: &str,
    attempt_count: i64,
    tenant_id: &str,
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
            // `channel` is the triage label on the dravr-outbound-dead-lettered
            // log metric (infra/environments/dev/turn_loss_monitoring.tf).
            warn!(
                entry_id = %entry_id,
                channel = %channel_type,
                "All retries exhausted, moving to dead-letter queue"
            );
            info!(
                target: "notify",
                event = "messaging.error",
                tenant_id = %tenant_id,
                channel = %channel_type,
                error_type = "dead_lettered",
                "outbound message dead-lettered"
            );
            let _ = db
                .update_outbound_status(entry_id, STATUS_DEAD_LETTER, update.attempt_count, None)
                .await;
        }
    }
}

/// Move an entry to the dead-letter queue
async fn dead_letter(db: &dyn MessagingRepository, entry_id: &str, attempt_count: i64) {
    let _ = db
        .update_outbound_status(
            entry_id,
            STATUS_DEAD_LETTER,
            i32::try_from(attempt_count + 1).unwrap_or(i32::MAX),
            None,
        )
        .await;
}
