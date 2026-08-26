// ABOUTME: Notification facade — the dravr-commere dispatcher plus the platform's own delivery sinks
// ABOUTME: Owns the NotificationChannelSink SPI so a notification can reach a linked chat channel

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Notifications
//!
//! Where the platform's own delivery sinks compose onto `dravr-commere`.
//!
//! The notification models, persistence, preference/quiet-hours/frequency
//! policy, Expo Push delivery and cron scheduling all live in that standalone
//! crate.
//!
//! `dravr-commere`'s dispatcher has exactly two sinks: it persists the
//! notification row, and it pushes to the user's Expo devices. An athlete who
//! talks to Dravr on Telegram, Slack or `WhatsApp` and has never installed the
//! mobile app therefore received *nothing* — not for social, not for training,
//! not for coach follow-ups. [`NotificationChannelSink`] is the seam that fixes
//! that for every category at once: [`NotificationService::dispatch`] runs the
//! upstream pipeline first, so preferences, quiet hours and frequency caps
//! decide as they always did, and delivers to the linked channel only when the
//! pipeline actually accepted the notification.
//!
//! The sink is an SPI rather than a direct dependency because the messaging
//! adapters, channel-link repository and localized string registry live above
//! this crate; the concrete implementation is
//! `pierre_services::notification_channel_sink::MessagingChannelSink`, wired at
//! startup. This mirrors how `pierre_services::provider_refresh` takes its push
//! notifier as a trait.

use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::errors::{AppError, AppResult};
use tracing::debug;

// Re-export all public modules from dravr-commere
pub use dravr_commere::constants;
pub use dravr_commere::expo_push;
pub use dravr_commere::models;

/// Event-shaped helpers that build a `DispatchRequest` and fire it through
/// [`NotificationService::dispatch`], so every product event reaches the
/// platform's sinks and not only the upstream two.
pub mod triggers;

// Re-export primary public types at crate root
pub use dravr_commere::{
    compute_next_fire_time, validate_cron_expression, CommereError, CommereResult, DispatchOutcome,
    DispatchRequest, SuppressionReason, TenantId,
};

/// A platform delivery sink for an accepted notification.
///
/// Implemented once, by the messaging sink in `pierre-services`, and consumed
/// once, by [`NotificationService::dispatch`]. Delivery is best-effort by
/// contract: a sink reports nothing and must not fail the dispatch, because the
/// notification is already persisted and visible in-app by the time a sink runs.
#[async_trait]
pub trait NotificationChannelSink: Send + Sync {
    /// Deliver `request` on whatever channels this sink owns.
    ///
    /// Called only for notifications the upstream pipeline accepted — never
    /// for one suppressed by category, quiet hours or a frequency cap.
    async fn deliver(&self, request: &DispatchRequest);
}

/// The platform's notification service.
///
/// `dravr-commere`'s pipeline plus the delivery sinks this platform adds on
/// top. [`Deref`]s to the upstream service, so device tokens, preferences,
/// scheduled notifications and analytics are reached exactly as before. Only
/// [`Self::dispatch`] is overridden — an inherent method takes precedence over
/// the deref'd one — which is what keeps the fan-out in one place instead of at
/// every call site that raises a notification.
pub struct NotificationService {
    /// The upstream pipeline: preferences, persistence, Expo push.
    inner: dravr_commere::NotificationService,
    /// Platform sinks that run after the pipeline accepts a notification.
    /// `None` when no channel sink is configured (messaging not compiled in,
    /// or no messaging channel configured for the deployment).
    channel_sink: Option<Arc<dyn NotificationChannelSink>>,
}

impl Deref for NotificationService {
    type Target = dravr_commere::NotificationService;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl NotificationService {
    /// Create a service backed by a `SQLite` database pool, with no channel
    /// sink attached. Add one with [`Self::with_channel_sink`].
    #[cfg(feature = "sqlite")]
    #[must_use]
    pub fn from_sqlite(pool: sqlx::SqlitePool) -> Self {
        Self {
            inner: dravr_commere::NotificationService::from_sqlite(pool),
            channel_sink: None,
        }
    }

    /// Create a service backed by a `PostgreSQL` database pool, with no channel
    /// sink attached. Add one with [`Self::with_channel_sink`].
    #[cfg(feature = "postgresql")]
    #[must_use]
    pub fn from_postgres(pool: sqlx::PgPool) -> Self {
        Self {
            inner: dravr_commere::NotificationService::from_postgres(pool),
            channel_sink: None,
        }
    }

    /// Attach the sink that delivers accepted notifications to a user's linked
    /// chat channels.
    #[must_use]
    pub fn with_channel_sink(mut self, sink: Arc<dyn NotificationChannelSink>) -> Self {
        self.channel_sink = Some(sink);
        self
    }

    /// Dispatch a notification through the full pipeline, then through every
    /// platform sink that accepted notification is entitled to.
    ///
    /// The upstream pipeline runs first and its verdict is authoritative: a
    /// [`DispatchOutcome::Suppressed`] means the user disabled the category, is
    /// inside quiet hours, or has hit the daily cap, and no sink runs. Anything
    /// else means the notification was persisted, so the sinks deliver it —
    /// including [`DispatchOutcome::PersistedNoDevices`], which is precisely
    /// the athlete who lives in a chat channel and has no mobile app.
    ///
    /// # Errors
    ///
    /// Returns the upstream [`CommereError`] when the pipeline itself fails.
    /// Sink failures are logged by the sink and never surface here.
    pub async fn dispatch(&self, request: &DispatchRequest) -> CommereResult<DispatchOutcome> {
        let outcome = self.inner.dispatch(request).await?;

        if matches!(outcome, DispatchOutcome::Suppressed(_)) {
            debug!(
                user_id = %request.user_id,
                category = %request.category,
                "Notification suppressed upstream; channel sink skipped"
            );
            return Ok(outcome);
        }

        if let Some(sink) = &self.channel_sink {
            sink.deliver(request).await;
        }

        Ok(outcome)
    }
}

/// Convert a `CommereError` to an `AppError` with structured mapping
#[must_use]
pub fn to_app_error(err: CommereError) -> AppError {
    match err {
        CommereError::Database(msg) => AppError::internal(msg),
        CommereError::PushDelivery { service, message } => {
            AppError::external_service(service, message)
        }
        CommereError::Validation { field, reason } => {
            AppError::invalid_input(format!("{field}: {reason}"))
        }
        CommereError::Scheduling(msg) => AppError::invalid_input(msg),
        CommereError::NotFound { resource } => AppError::not_found(resource),
    }
}

/// Convert a `CommereResult<T>` to an `AppResult<T>`
///
/// # Errors
/// Returns `AppError` mapped from the underlying `CommereError`.
pub fn to_app_result<T>(result: CommereResult<T>) -> AppResult<T> {
    result.map_err(to_app_error)
}
