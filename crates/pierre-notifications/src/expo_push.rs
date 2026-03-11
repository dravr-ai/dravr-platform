// ABOUTME: Expo Push Service client for sending push notifications via Expo's HTTP API
// ABOUTME: Handles batch sending, receipt tracking, and error handling for iOS/Android push delivery
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Expo Push Notification Service
//!
//! Sends push notifications to registered devices via the Expo Push API.
//! Expo routes notifications to APNs (iOS) and FCM (Android) transparently.
//! Supports batch sending up to 100 notifications per request.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::api_client;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::slice;
use tracing::{info, warn};

/// Expo push API endpoint
const EXPO_PUSH_URL: &str = "https://exp.host/--/api/v2/push/send";
/// Expo push receipts endpoint
const EXPO_RECEIPTS_URL: &str = "https://exp.host/--/api/v2/push/getReceipts";
/// Maximum notifications per batch request
const MAX_BATCH_SIZE: usize = 100;
/// Expo push notification message payload
#[derive(Debug, Clone, Serialize)]
pub struct ExpoPushMessage {
    /// Expo push token (e.g. `ExponentPushToken[...]`)
    pub to: String,
    /// Notification title
    pub title: String,
    /// Notification body text
    pub body: String,
    /// JSON data payload for deep-linking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    /// Sound to play (use "default" for system default)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    /// Badge count to set on the app icon (iOS)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<u32>,
    /// Notification channel ID (Android)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Category identifier for notification actions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<String>,
    /// Priority: "default", "normal", or "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// Response from Expo push API for a single notification
#[derive(Debug, Clone, Deserialize)]
pub struct ExpoPushTicket {
    /// Status: "ok" or "error"
    pub status: String,
    /// Ticket ID for receipt tracking (present when status is "ok")
    pub id: Option<String>,
    /// Error message (present when status is "error")
    pub message: Option<String>,
    /// Error details
    pub details: Option<serde_json::Value>,
}

/// Top-level response from Expo push API
#[derive(Debug, Clone, Deserialize)]
pub struct ExpoPushResponse {
    /// Array of tickets, one per message sent
    pub data: Vec<ExpoPushTicket>,
}

/// Receipt for a previously sent notification
#[derive(Debug, Clone, Deserialize)]
pub struct ExpoPushReceipt {
    /// Status: "ok" or "error"
    pub status: String,
    /// Error message
    pub message: Option<String>,
    /// Error details
    pub details: Option<serde_json::Value>,
}

/// Receipt response from Expo
#[derive(Debug, Clone, Deserialize)]
pub struct ExpoReceiptResponse {
    /// Map of ticket ID to receipt
    pub data: HashMap<String, ExpoPushReceipt>,
}

/// Expo Push Service client
pub struct ExpoPushService {
    /// Shared HTTP client for API requests
    client: &'static Client,
}

impl Default for ExpoPushService {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpoPushService {
    /// Create a new Expo Push Service using the shared HTTP client
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: api_client(),
        }
    }

    /// Send a single push notification
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or Expo returns an error response
    pub async fn send(&self, message: &ExpoPushMessage) -> AppResult<ExpoPushTicket> {
        let tickets = self.send_batch(slice::from_ref(message)).await?;
        tickets
            .into_iter()
            .next()
            .ok_or_else(|| AppError::external_service("Expo", "Empty response from push API"))
    }

    /// Send a batch of push notifications (up to 100 per request)
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails or Expo returns an error response
    pub async fn send_batch(&self, messages: &[ExpoPushMessage]) -> AppResult<Vec<ExpoPushTicket>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_tickets = Vec::with_capacity(messages.len());

        // Split into batches of MAX_BATCH_SIZE
        for chunk in messages.chunks(MAX_BATCH_SIZE) {
            let response = self
                .client
                .post(EXPO_PUSH_URL)
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .json(&chunk)
                .send()
                .await
                .map_err(|e| {
                    AppError::external_service("Expo", format!("Push request failed: {e}"))
                })?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                warn!(
                    status = %status,
                    body = %body,
                    "Expo push API returned error"
                );
                return Err(AppError::external_service(
                    "Expo",
                    format!("Push API returned HTTP {status}: {body}"),
                ));
            }

            let push_response: ExpoPushResponse = response.json().await.map_err(|e| {
                AppError::external_service("Expo", format!("Failed to parse push response: {e}"))
            })?;

            info!(
                batch_size = chunk.len(),
                tickets = push_response.data.len(),
                "Expo push batch sent"
            );

            all_tickets.extend(push_response.data);
        }

        Ok(all_tickets)
    }

    /// Fetch receipts for previously sent notifications
    ///
    /// # Errors
    /// Returns an error if the HTTP request fails
    pub async fn get_receipts(
        &self,
        ticket_ids: &[String],
    ) -> AppResult<HashMap<String, ExpoPushReceipt>> {
        if ticket_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let body = serde_json::json!({ "ids": ticket_ids });

        let response = self
            .client
            .post(EXPO_RECEIPTS_URL)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::external_service("Expo", format!("Receipt request failed: {e}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::external_service(
                "Expo",
                format!("Receipt API returned HTTP {status}: {body}"),
            ));
        }

        let receipt_response: ExpoReceiptResponse = response.json().await.map_err(|e| {
            AppError::external_service("Expo", format!("Failed to parse receipt response: {e}"))
        })?;

        Ok(receipt_response.data)
    }
}
