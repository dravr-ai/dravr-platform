// ABOUTME: Terra webhook handler for receiving push data from 150+ wearables
// ABOUTME: Validates signatures, parses payloads into activities and wellness records for the platform to store
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Terra webhook handler
//!
//! This module handles incoming webhook events from Terra. When users sync their
//! wearables, Terra pushes data to your configured webhook endpoint.
//!
//! The handler parses a payload into [`TerraRecords`]: the activities and the
//! wellness records (nights, recovery days, body measurements, daily activity
//! totals) it carries, in the shapes Dravr's stores keep. It stores nothing:
//! the platform writes the records through the one activity-cache writer and
//! the wellness store, for the athlete the Terra connection belongs to.
//! Nutrition has no store, so a nutrition event is parsed and dropped.
//!
//! ## Security
//!
//! All webhook requests include a `terra-signature` header of the form
//! `t=<timestamp>,v1=<signature>`, where each `v1` value is the hex HMAC-SHA256 of
//! `"{timestamp}.{raw body}"` keyed by the endpoint signing secret. Only the `v1`
//! scheme is trusted, and the header may carry more than one `v1` value. This
//! handler validates signatures before processing any data.
//!
//! ## Event Types
//!
//! - `activity` - Workout/activity data
//! - `sleep` - Sleep session data
//! - `body` - Body metrics (weight, body fat, etc.)
//! - `daily` - Daily activity summaries
//! - `nutrition` - Nutrition/food log data
//! - `auth` - Authentication events (user connected/disconnected)

use pierre_core::constant_time::{is_unset_secret, matches_configured_secret};
use ring::hmac;
use serde::de::DeserializeOwned;
use std::fmt;
use tracing::{debug, error, info, warn};

use super::converters::TerraConverters;
use super::models::{
    TerraActivity, TerraBody, TerraDaily, TerraSleep, TerraUser, TerraWebhookPayload,
};
use crate::models::{
    Activity, StoredDailyActivity, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession,
};

/// Webhook signature validation result
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureValidation {
    /// Signature is valid
    Valid,
    /// Signature is invalid
    Invalid,
    /// Signature header is missing
    Missing,
    /// No signing secret configured — validation cannot be performed
    NotConfigured,
}

/// Validates Terra webhook signatures
pub struct WebhookSignatureValidator {
    /// Terra webhook signing secret
    signing_secret: String,
}

impl fmt::Debug for WebhookSignatureValidator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookSignatureValidator")
            .field("signing_secret", &"[REDACTED]")
            .finish()
    }
}

impl WebhookSignatureValidator {
    /// Create a new signature validator
    #[must_use]
    pub const fn new(signing_secret: String) -> Self {
        Self { signing_secret }
    }

    /// Validate a webhook request signature
    ///
    /// The request is valid when any `v1` signature in the header equals the
    /// HMAC-SHA256 of `"{t}.{body}"` keyed by the signing secret. A header without
    /// a `t` timestamp or without a `v1` signature is invalid.
    ///
    /// Under an unset signing secret (empty or whitespace only,
    /// `is_unset_secret`) every signature is invalid: anyone can compute an
    /// HMAC under the empty key or a lone newline, so one made with it proves
    /// no one.
    ///
    /// # Arguments
    /// * `signature_header` - Value of the `terra-signature` header
    /// * `body` - Raw, unaltered request body bytes
    ///
    /// # Returns
    /// `SignatureValidation` indicating whether the signature is valid
    #[must_use]
    pub fn validate(&self, signature_header: Option<&str>, body: &[u8]) -> SignatureValidation {
        if is_unset_secret(self.signing_secret.as_bytes()) {
            return SignatureValidation::Invalid;
        }
        let Some(header) = signature_header else {
            return SignatureValidation::Missing;
        };
        let Some(parsed) = SignatureHeader::parse(header) else {
            return SignatureValidation::Invalid;
        };

        let expected = self.expected_signature(parsed.timestamp, body);
        let matched = parsed.signatures.iter().any(|signature| {
            hex::decode(signature)
                .is_ok_and(|presented| matches_configured_secret(expected.as_ref(), &presented))
        });

        if matched {
            SignatureValidation::Valid
        } else {
            SignatureValidation::Invalid
        }
    }

    /// HMAC-SHA256 of Terra's signed payload `"{timestamp}.{body}"`, keyed by the
    /// signing secret
    ///
    /// The payload is fed to the MAC in pieces, so the body is hashed once and
    /// never copied however many `v1` signatures the header carries.
    fn expected_signature(&self, timestamp: &str, body: &[u8]) -> hmac::Tag {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.signing_secret.as_bytes());
        let mut context = hmac::Context::with_key(&key);
        context.update(timestamp.as_bytes());
        context.update(b".");
        context.update(body);
        context.sign()
    }
}

/// Parsed `terra-signature` header: the signed timestamp and every `v1` signature
struct SignatureHeader<'a> {
    /// Value of the `t` element, signed verbatim as the payload prefix
    timestamp: &'a str,
    /// Hex values of every `v1` element
    signatures: Vec<&'a str>,
}

impl<'a> SignatureHeader<'a> {
    /// Parse `t=<timestamp>,v1=<hex>[,v1=<hex>...]`
    ///
    /// Elements are split on `,` and then on `=`, with surrounding whitespace
    /// trimmed. The first `t` is the timestamp. Every scheme other than `v1` is
    /// dropped, so a weaker scheme such as `v0` can never stand in for `v1`.
    /// Returns `None` when the timestamp is absent or empty, or when no `v1`
    /// signature is present.
    fn parse(header: &'a str) -> Option<Self> {
        let mut timestamp = None;
        let mut signatures = Vec::new();
        for element in header.split(',') {
            match element.trim().split_once('=') {
                Some(("t", value)) if timestamp.is_none() => timestamp = Some(value),
                Some(("v1", value)) => signatures.push(value),
                _ => {}
            }
        }

        let timestamp = timestamp.filter(|value| !value.is_empty())?;
        (!signatures.is_empty()).then_some(Self {
            timestamp,
            signatures,
        })
    }
}

/// What one data event carried, in the shapes Dravr's stores keep.
///
/// The wellness records are unstamped: they name no user or data source
/// until the platform files them for the connection's owner.
#[derive(Debug, Clone, Default)]
pub struct TerraRecords {
    /// Workouts, as the activity cache keeps them (provider `terra`, each
    /// naming the service that recorded it as its source).
    pub activities: Vec<Activity>,
    /// Nights.
    pub nights: Vec<StoredSleepSession>,
    /// Recovery readings per local date, from sleep and daily summaries.
    pub recovery: Vec<StoredRecoveryMetrics>,
    /// Body measurements per local date.
    pub body: Vec<StoredHealthMetrics>,
    /// Activity totals per local date.
    pub daily_activity: Vec<StoredDailyActivity>,
}

impl TerraRecords {
    /// How many records the event carried.
    #[must_use]
    pub fn len(&self) -> usize {
        self.activities.len()
            + self.nights.len()
            + self.recovery.len()
            + self.body.len()
            + self.daily_activity.len()
    }

    /// Whether the event carried no record.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Result of processing a webhook event
#[derive(Debug, Clone)]
pub enum WebhookResult {
    /// A data event, parsed into the records it carried
    Success {
        /// Event type that was processed
        event_type: String,
        /// Terra user ID
        user_id: String,
        /// The service Terra relayed the data from (`GARMIN`, `OURA`, ...),
        /// when the payload names it
        relayed_from: Option<String>,
        /// What the event carried
        records: Box<TerraRecords>,
    },
    /// Authentication event (user connected/disconnected)
    AuthEvent {
        /// Event type (auth, deauth, `user_reauth`)
        event_type: String,
        /// Status message
        status: String,
        /// Terra user ID (if available)
        user_id: Option<String>,
        /// Reference ID (if available)
        reference_id: Option<String>,
    },
    /// Unknown or unhandled event type, or one Dravr keeps nothing of
    /// (nutrition)
    Unhandled {
        /// Event type
        event_type: String,
    },
    /// Error processing webhook
    Error {
        /// Error message
        message: String,
    },
}

/// The summaries of `items` in the shape `T` their event type names; a
/// summary that does not parse is skipped with a warning.
fn summaries<T: DeserializeOwned>(items: &[serde_json::Value], event_type: &str) -> Vec<T> {
    items
        .iter()
        .filter_map(|item| {
            T::deserialize(item)
                .map_err(|e| warn!("A Terra {event_type} summary did not parse: {e}"))
                .ok()
        })
        .collect()
}

/// Terra webhook handler
///
/// Validates and parses incoming webhook events from Terra into the records
/// they carry; storing them is the platform's.
#[derive(Debug, Default)]
pub struct TerraWebhookHandler {
    validator: Option<WebhookSignatureValidator>,
}

impl TerraWebhookHandler {
    /// Create a new webhook handler
    #[must_use]
    pub const fn new() -> Self {
        Self { validator: None }
    }

    /// Create a webhook handler with signature validation
    #[must_use]
    pub const fn with_validation(signing_secret: String) -> Self {
        Self {
            validator: Some(WebhookSignatureValidator::new(signing_secret)),
        }
    }

    /// Validate a webhook request signature
    #[must_use]
    pub fn validate_signature(
        &self,
        signature_header: Option<&str>,
        body: &[u8],
    ) -> SignatureValidation {
        self.validator
            .as_ref()
            .map_or(SignatureValidation::NotConfigured, |v| {
                v.validate(signature_header, body)
            })
    }

    /// Parse webhook payload from raw bytes
    fn parse_payload(body: &[u8]) -> Result<TerraWebhookPayload, WebhookResult> {
        serde_json::from_slice(body).map_err(|e| {
            error!("Failed to parse Terra webhook payload: {}", e);
            WebhookResult::Error {
                message: format!("JSON parse error: {e}"),
            }
        })
    }

    /// Check if event type is an authentication event
    fn is_auth_event(event_type: &str) -> bool {
        matches!(event_type, "auth" | "deauth" | "user_reauth")
    }

    /// Extract and validate user from payload
    fn extract_user(payload: &TerraWebhookPayload) -> Result<&TerraUser, WebhookResult> {
        payload.user.as_ref().ok_or_else(|| {
            warn!(
                "Terra webhook missing user field for event type: {}",
                payload.event_type
            );
            WebhookResult::Error {
                message: "Missing user field in payload".to_owned(),
            }
        })
    }

    /// The records a data event of `event_type` carries, or the result that
    /// answers an event Dravr keeps nothing of.
    fn records_of(
        payload: &TerraWebhookPayload,
        user: &TerraUser,
        event_type: &str,
    ) -> Result<TerraRecords, WebhookResult> {
        let items = payload.data.as_deref().unwrap_or_default();
        let mut records = TerraRecords::default();
        match event_type {
            "activity" => {
                for terra in summaries::<TerraActivity>(items, event_type) {
                    records
                        .activities
                        .extend(TerraConverters::activity_from_terra(&terra, user));
                }
            }
            "sleep" => {
                for terra in summaries::<TerraSleep>(items, event_type) {
                    records
                        .nights
                        .extend(TerraConverters::night_from_terra(&terra));
                    records
                        .recovery
                        .extend(TerraConverters::night_recovery_from_terra(&terra));
                }
            }
            "body" => {
                for terra in summaries::<TerraBody>(items, event_type) {
                    records
                        .body
                        .extend(TerraConverters::body_from_terra(&terra));
                }
            }
            "daily" => {
                for terra in summaries::<TerraDaily>(items, event_type) {
                    records
                        .recovery
                        .extend(TerraConverters::day_recovery_from_terra(&terra));
                    records
                        .daily_activity
                        .extend(TerraConverters::day_activity_from_terra(&terra));
                }
            }
            _ => {
                info!("Unhandled Terra webhook event type: {}", event_type);
                return Err(WebhookResult::Unhandled {
                    event_type: event_type.to_owned(),
                });
            }
        }
        Ok(records)
    }

    /// Process a webhook payload
    ///
    /// # Arguments
    /// * `body` - Raw JSON body of the webhook request
    ///
    /// # Returns
    /// `WebhookResult` indicating the outcome of processing, with the records
    /// a data event carried
    pub fn process(&self, body: &[u8]) -> WebhookResult {
        let payload = match Self::parse_payload(body) {
            Ok(p) => p,
            Err(e) => return e,
        };

        let event_type = payload.event_type.clone();
        debug!("Processing Terra webhook event: {}", event_type);

        if Self::is_auth_event(&event_type) {
            return Self::handle_auth_event(&payload);
        }

        let user = match Self::extract_user(&payload) {
            Ok(u) => u,
            Err(e) => return e,
        };

        let records = match Self::records_of(&payload, user, &event_type) {
            Ok(records) => records,
            Err(result) => return result,
        };

        info!(
            "Parsed {} {} records for Terra user {}",
            records.len(),
            event_type,
            user.user_id
        );

        WebhookResult::Success {
            event_type,
            user_id: user.user_id.clone(),
            relayed_from: user.provider.clone(),
            records: Box::new(records),
        }
    }

    /// Handle authentication events
    fn handle_auth_event(payload: &TerraWebhookPayload) -> WebhookResult {
        let event_type = payload.event_type.clone();
        let status = payload
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());

        let user_id = payload.user.as_ref().map(|u| u.user_id.clone());
        let reference_id = payload.user.as_ref().and_then(|u| u.reference_id.clone());

        info!(
            "Terra auth event: {} - status: {} - user: {:?}",
            event_type, status, user_id
        );

        WebhookResult::AuthEvent {
            event_type,
            status,
            user_id,
            reference_id,
        }
    }
}
