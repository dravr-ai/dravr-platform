// ABOUTME: A2A client registration, credential, and usage data structures
// ABOUTME: Standalone types for client management and the client's request budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, NaiveDate, Utc};
use pierre_auth::rate_limiting::{a2a_client_window_resets_at, calculate_a2a_client_rate_limit};
use pierre_core::models::a2a::A2AClient;
use pierre_core::models::WindowUsage;
use serde::{Deserialize, Serialize};

/// A2A Client registration request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRegistrationRequest {
    /// Name of the client application
    pub name: String,
    /// Description of the client's purpose
    pub description: String,
    /// List of agent capabilities this client provides
    pub capabilities: Vec<String>,
    /// `OAuth2` redirect URIs for authorization flows
    pub redirect_uris: Vec<String>,
    /// Contact email for the client administrator
    pub contact_email: String,
}

/// A2A Client credentials response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCredentials {
    /// Unique client identifier
    pub client_id: String,
    /// Client secret for authentication
    pub client_secret: String,
    /// API key for direct API access
    pub api_key: String,
    /// Ed25519 public key for signature verification
    pub public_key: String,
    /// Ed25519 private key for signing (client-side only, never stored)
    pub private_key: String,
    /// Key type identifier ("ed25519")
    pub key_type: String,
}

/// A2A client usage statistics, counted over the client's `a2a_usage` rows
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClientUsageStats {
    /// Client identifier
    pub client_id: String,
    /// Calls since the start of the UTC day
    pub requests_today: u64,
    /// Calls since the start of the UTC month
    pub requests_this_month: u64,
    /// Every call the client has made
    pub total_requests: u64,
    /// Timestamp of the most recent call
    pub last_request_at: Option<DateTime<Utc>>,
    /// One row per UTC calendar day that saw a call, newest first
    pub daily_usage: Vec<DailyUsage>,
}

/// A client's calls on one UTC calendar day
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DailyUsage {
    /// The UTC calendar day
    pub date: NaiveDate,
    /// Calls answered below 400
    pub success_count: u32,
    /// Calls answered 400 or above
    pub error_count: u32,
}

/// An A2A client's request budget as its owner sees it: the client row's
/// `rate_limit_requests` over a sliding `rate_limit_window_seconds`, the
/// budget a client-credentials call spends and is refused on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct A2ARateLimitStatus {
    /// Client identifier
    pub client_id: String,
    /// Whether the window has already admitted its limit, so the next call
    /// is refused
    pub is_rate_limited: bool,
    /// Calls the window admits
    pub rate_limit_requests: u32,
    /// The length of the sliding window, in seconds
    pub rate_limit_window_seconds: u32,
    /// Calls counted inside the window
    pub current_usage: u32,
    /// Calls the window still admits
    pub remaining: u32,
    /// When the window frees its first slot
    pub reset_at: DateTime<Utc>,
}

impl A2ARateLimitStatus {
    /// The status of `client`'s budget, given the calls `usage` counted
    /// inside its window at `now`.
    #[must_use]
    pub fn from_window(client: &A2AClient, usage: &WindowUsage, now: DateTime<Utc>) -> Self {
        Self {
            client_id: client.id.clone(),
            is_rate_limited: calculate_a2a_client_rate_limit(client, usage, now).is_exceeded(),
            rate_limit_requests: client.rate_limit_requests,
            rate_limit_window_seconds: client.rate_limit_window_seconds,
            current_usage: usage.count,
            remaining: client.rate_limit_requests.saturating_sub(usage.count),
            reset_at: a2a_client_window_resets_at(client, usage, now),
        }
    }
}
