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
//! mobile app therefore received *nothing* — not for training, not for recovery,
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
use serde_json::json;
use tracing::{debug, info};

// Re-export all public modules from dravr-commere
pub use dravr_commere::constants;
pub use dravr_commere::expo_push;
pub use dravr_commere::models;

/// The closed product-event vocabulary a notification row records, and the
/// catalogue keys the sentence is rendered from at read time.
pub mod events;

/// The persona push-tier ladder, per-user push policy, and the
/// [`PersonaPolicyGate`] SPI the dispatch facade resolves policies through.
pub mod policy;

/// Event-shaped helpers that declare a product event and fire it through
/// [`NotificationService::dispatch_event`], so every product event reaches
/// the platform's sinks and not only the upstream two.
pub mod triggers;

// Re-export primary public types at crate root
pub use dravr_commere::{
    compute_next_fire_time, validate_cron_expression, CommereError, CommereResult, DispatchOutcome,
    DispatchRequest, SuppressionReason, TenantId,
};
pub use events::{EventDispatch, NotificationActionSpec, NotificationEvent};
pub use policy::{DigestCadence, PersonaPolicyGate, PushPolicy, PushTier};

/// JSON key marking a persisted notification the persona policy withheld from
/// push. The weekly digest scheduler collects rows carrying this marker; the
/// in-app list shows them like any other notification.
pub const PERSONA_GATED_DATA_KEY: &str = "persona_gated";

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

/// The user-facing text of one notification, rendered in one locale.
#[derive(Debug, Clone)]
pub struct NotificationText {
    /// The notification headline.
    pub title: String,
    /// The notification body.
    pub body: String,
    /// Action-button labels, in the order [`EventDispatch::actions`] declares
    /// them. Empty when the event attaches no buttons.
    pub action_titles: Vec<String>,
}

impl NotificationText {
    /// The catalogue keys themselves, for a service assembled without a
    /// localizer.
    ///
    /// Such a service holds no string catalogue and no way to read the
    /// recipient's locale, so the only honest thing it can persist is the key
    /// naming the sentence. The notification centre renders the row from its
    /// event and parameters regardless of what the columns hold, so this text
    /// only ever reaches an Expo push — which no deployment sends without a
    /// localizer, since the composition root wires one unconditionally.
    #[must_use]
    pub fn keys(dispatch: &EventDispatch) -> Self {
        Self {
            title: dispatch.event.title_key().to_owned(),
            body: dispatch.event.body_key().to_owned(),
            action_titles: dispatch.actions.as_ref().map_or_else(Vec::new, |actions| {
                actions
                    .iter()
                    .map(|action| {
                        events::action_label_key(action.id)
                            .unwrap_or(action.id)
                            .to_owned()
                    })
                    .collect()
            }),
        }
    }
}

/// Renders a product event as the sentence one recipient reads.
///
/// Implemented once, by `pierre_services::notification_localizer`, and
/// consumed once, by [`NotificationService::dispatch_event`]. It is an SPI
/// rather than a direct dependency for the same reason
/// [`NotificationChannelSink`] is: the string catalogue and the user
/// repository that holds each athlete's locale both live above this crate.
#[async_trait]
pub trait NotificationLocalizer: Send + Sync {
    /// The title, body and action labels for `dispatch`, in the recipient's
    /// stored locale.
    async fn localize(&self, dispatch: &EventDispatch) -> NotificationText;
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
    /// Resolves the per-user persona push policy. `None` when persona gating
    /// is not wired (bare test services); every dispatch then behaves as if
    /// no policy existed.
    policy_gate: Option<Arc<dyn PersonaPolicyGate>>,
    /// Renders each event as the recipient's own sentence. `None` when no
    /// string catalogue is wired (bare test services); the row then carries
    /// the catalogue keys, which the notification centre resolves anyway.
    localizer: Option<Arc<dyn NotificationLocalizer>>,
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
            policy_gate: None,
            localizer: None,
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
            policy_gate: None,
            localizer: None,
        }
    }

    /// Attach the sink that delivers accepted notifications to a user's linked
    /// chat channels.
    #[must_use]
    pub fn with_channel_sink(mut self, sink: Arc<dyn NotificationChannelSink>) -> Self {
        self.channel_sink = Some(sink);
        self
    }

    /// Attach the gate that resolves each recipient's persona push policy.
    #[must_use]
    pub fn with_policy_gate(mut self, gate: Arc<dyn PersonaPolicyGate>) -> Self {
        self.policy_gate = Some(gate);
        self
    }

    /// Attach the renderer that turns a product event into the recipient's
    /// own sentence.
    #[must_use]
    pub fn with_localizer(mut self, localizer: Arc<dyn NotificationLocalizer>) -> Self {
        self.localizer = Some(localizer);
        self
    }

    /// Dispatch a product event at an explicit [`PushTier`].
    ///
    /// This is how every trigger raises a notification: it declares what
    /// happened and the parameters that describe it, the localizer renders
    /// the sentence in the recipient's language — so the Expo push and the
    /// linked chat channels read correctly the first time — and the stored
    /// row keeps the event plus its parameters so the notification centre can
    /// render it again when the athlete changes language.
    ///
    /// # Errors
    ///
    /// Returns the upstream [`CommereError`] when the pipeline itself fails.
    pub async fn dispatch_event(
        &self,
        dispatch: &EventDispatch,
        tier: PushTier,
    ) -> CommereResult<DispatchOutcome> {
        let text = match &self.localizer {
            Some(localizer) => localizer.localize(dispatch).await,
            None => NotificationText::keys(dispatch),
        };
        let actions = dispatch.actions.as_ref().map(|specs| {
            specs
                .iter()
                .zip(text.action_titles.iter())
                .map(|(spec, title)| models::NotificationAction {
                    id: spec.id.to_owned(),
                    title: title.clone(),
                    action_type: spec.action_type.clone(),
                })
                .collect()
        });
        let request = DispatchRequest {
            user_id: dispatch.user_id,
            tenant_id: dispatch.tenant_id,
            category: dispatch.category,
            notification_type: dispatch.event.wire().to_owned(),
            title: text.title,
            body: text.body,
            data: Some(events::event_data(
                dispatch.route.clone(),
                dispatch.params.clone(),
            )),
            image_url: None,
            actions,
            bypass_frequency_cap: dispatch.bypass_frequency_cap,
        };
        self.dispatch_with_tier(&request, tier).await
    }

    /// Dispatch a notification at the default [`PushTier::P1`].
    ///
    /// P1 is the conservative default for a call site that has not declared a
    /// tier: it delivers to every persona except the strictest floor (Casual's
    /// P0), so an unmigrated caller behaves like the high-signal events rather
    /// than sneaking past every floor as P0 would. Call sites that know their
    /// product semantics use [`Self::dispatch_with_tier`] directly.
    ///
    /// # Errors
    ///
    /// Returns the upstream [`CommereError`] when the pipeline itself fails.
    pub async fn dispatch(&self, request: &DispatchRequest) -> CommereResult<DispatchOutcome> {
        self.dispatch_with_tier(request, PushTier::P1).await
    }

    /// Dispatch a notification at an explicit [`PushTier`] through the persona
    /// gate, the upstream pipeline, and every platform sink.
    ///
    /// The persona gate runs first. When the recipient's policy is **armed**
    /// and the event's tier falls above their floor (floor `Pn` delivers tiers
    /// ≤ `Pn` only), the notification is persisted directly — visible in-app
    /// and collectible by the weekly digest, its `data` carrying
    /// [`PERSONA_GATED_DATA_KEY`] — and neither Expo push nor the channel sink
    /// runs. The returned [`DispatchOutcome::PersistedNoDevices`] is then
    /// indistinguishable from an ungated dispatch to a device-less user: the
    /// true verdict (gated vs no-devices) lives in this method's structured
    /// logs, not in the outcome.
    ///
    /// When the policy is **not armed** (shadow mode), a structured
    /// shadow-verdict log records what enforcement would have done and the
    /// dispatch proceeds untouched.
    ///
    /// Past the gate, the upstream pipeline's verdict is authoritative: a
    /// [`DispatchOutcome::Suppressed`] means the user disabled the category, is
    /// inside quiet hours, or has hit the daily cap, and no sink runs. Anything
    /// else means the notification was persisted, so the sinks deliver it —
    /// including [`DispatchOutcome::PersistedNoDevices`], which is precisely
    /// the athlete who lives in a chat channel and has no mobile app.
    ///
    /// # Errors
    ///
    /// Returns the upstream [`CommereError`] when persistence or the pipeline
    /// fails. Sink failures are logged by the sink and never surface here.
    pub async fn dispatch_with_tier(
        &self,
        request: &DispatchRequest,
        tier: PushTier,
    ) -> CommereResult<DispatchOutcome> {
        if let Some(gate) = &self.policy_gate {
            if let Some(push_policy) = gate.policy_for(request.user_id, request.tenant_id).await {
                let would_gate = push_policy.gates(tier);
                if push_policy.armed && would_gate {
                    return self.persist_gated(request, tier, &push_policy).await;
                }
                if !push_policy.armed {
                    info!(
                        user_id = %request.user_id,
                        persona = %push_policy.persona,
                        notification_type = %request.notification_type,
                        event_tier = %tier,
                        floor = ?push_policy.floor,
                        would_gate,
                        "persona notification policy shadow verdict"
                    );
                }
            }
        }
        self.deliver(request).await
    }

    /// Persist a persona-gated notification without running the pipeline's
    /// push path or the channel sink. The row is what the weekly digest and
    /// the in-app list read; the `persona_gated` marker in `data` is how the
    /// digest scheduler finds it.
    async fn persist_gated(
        &self,
        request: &DispatchRequest,
        tier: PushTier,
        push_policy: &PushPolicy,
    ) -> CommereResult<DispatchOutcome> {
        // Every shipping call site passes an object or None; a non-object
        // payload is preserved under "payload" so the marker never destroys
        // caller data.
        let data = request.data.clone().map_or_else(
            || json!({ PERSONA_GATED_DATA_KEY: true }),
            |mut value| {
                if let Some(object) = value.as_object_mut() {
                    object.insert(PERSONA_GATED_DATA_KEY.to_owned(), json!(true));
                    value
                } else {
                    json!({ PERSONA_GATED_DATA_KEY: true, "payload": value })
                }
            },
        );
        let params = models::CreateNotificationParams {
            user_id: request.user_id,
            tenant_id: request.tenant_id,
            category: request.category,
            notification_type: request.notification_type.clone(),
            title: request.title.clone(),
            body: request.body.clone(),
            data: Some(data),
            image_url: request.image_url.clone(),
            actions: request.actions.clone(),
        };
        let notification = self.inner.create_notification(&params).await?;
        info!(
            user_id = %request.user_id,
            persona = %push_policy.persona,
            notification_type = %request.notification_type,
            event_tier = %tier,
            floor = ?push_policy.floor,
            notification_id = %notification.id,
            "persona notification policy gated a push; persisted for the digest"
        );
        Ok(DispatchOutcome::PersistedNoDevices {
            notification_id: notification.id,
        })
    }

    /// Run the upstream pipeline, then the channel sink when it accepted.
    async fn deliver(&self, request: &DispatchRequest) -> CommereResult<DispatchOutcome> {
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
