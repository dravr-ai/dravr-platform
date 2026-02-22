// ABOUTME: A2A client registration, credential, and usage data structures
// ABOUTME: Standalone types for client management and rate limiting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
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

/// A2A Client usage statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientUsageStats {
    /// Client identifier
    pub client_id: String,
    /// Number of requests made today
    pub requests_today: u64,
    /// Number of requests made this month
    pub requests_this_month: u64,
    /// Total number of requests ever made
    pub total_requests: u64,
    /// Timestamp of the most recent request
    pub last_request_at: Option<DateTime<Utc>>,
    /// Current rate limit tier
    pub rate_limit_tier: String,
}

/// A2A Client rate limit tiers
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum A2AClientTier {
    /// Trial tier - 1,000 requests/month, auto-expires in 30 days
    #[default]
    Trial,
    /// Standard tier - 10,000 requests/month
    Standard,
    /// Professional tier - 100,000 requests/month
    Professional,
    /// Enterprise tier - Unlimited requests
    Enterprise,
}

impl A2AClientTier {
    /// Returns the monthly API call limit for this tier
    ///
    /// Returns `None` for Enterprise tier (unlimited)
    #[must_use]
    pub const fn monthly_limit(&self) -> Option<u32> {
        match self {
            Self::Trial => Some(1000),
            Self::Standard => Some(10000),
            Self::Professional => Some(100_000),
            Self::Enterprise => None, // Unlimited
        }
    }

    /// Returns the human-readable display name for this tier
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Trial => "Trial",
            Self::Standard => "Standard",
            Self::Professional => "Professional",
            Self::Enterprise => "Enterprise",
        }
    }
}

/// A2A Rate limit status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ARateLimitStatus {
    /// Whether the client is currently rate limited
    pub is_rate_limited: bool,
    /// Maximum requests allowed in the current period
    pub limit: Option<u32>,
    /// Remaining requests in the current period
    pub remaining: Option<u32>,
    /// When the rate limit period resets
    pub reset_at: Option<DateTime<Utc>>,
    /// Current rate limit tier
    pub tier: A2AClientTier,
}

/// Parameters for detailed A2A usage recording
#[derive(Debug, Clone)]
pub struct A2AUsageParams {
    /// Client identifier
    pub client_id: String,
    /// Session token if available
    pub session_token: Option<String>,
    /// Name of the tool/endpoint called
    pub tool_name: String,
    /// Response time in milliseconds
    pub response_time_ms: Option<u32>,
    /// HTTP status code returned
    pub status_code: u16,
    /// Error message if the request failed
    pub error_message: Option<String>,
    /// Size of the request in bytes
    pub request_size_bytes: Option<u32>,
    /// Size of the response in bytes
    pub response_size_bytes: Option<u32>,
    /// Client IP address
    pub ip_address: Option<String>,
    /// Client user agent string
    pub user_agent: Option<String>,
    /// Capabilities advertised by the client
    pub client_capabilities: Vec<String>,
    /// `OAuth2` scopes granted for this request
    pub granted_scopes: Vec<String>,
}

/// A2A Authentication token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AToken {
    /// A2A client identifier
    pub client_id: String,
    /// User ID associated with this token
    pub user_id: String,
    /// List of OAuth scopes granted to this token
    pub scopes: Vec<String>,
    /// When this token expires
    pub expires_at: DateTime<Utc>,
    /// When this token was created
    pub created_at: DateTime<Utc>,
}
