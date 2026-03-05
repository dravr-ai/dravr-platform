// ABOUTME: Structured error types for messaging gateway operations using thiserror
// ABOUTME: Provides domain-specific errors for signature verification, delivery, and session management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use thiserror::Error;

/// Messaging gateway operation errors with structured context
#[derive(Error, Debug)]
pub enum MessagingError {
    /// Webhook signature verification failed (HMAC, Ed25519, or secret token mismatch)
    #[error("Signature verification failed for {channel}: {reason}")]
    SignatureVerificationFailed {
        /// Channel that failed verification
        channel: String,
        /// Reason for failure
        reason: String,
    },

    /// Webhook timestamp is outside acceptable window (replay protection)
    #[error("Replay detected for {channel}: {reason}")]
    ReplayDetected {
        /// Channel where replay was detected
        channel: String,
        /// Explanation of the timing violation
        reason: String,
    },

    /// Duplicate `channel_message_id` already processed (idempotency guard)
    #[error("Duplicate message {channel_message_id} on {channel}")]
    DuplicateMessage {
        /// Channel type
        channel: String,
        /// The duplicate message identifier
        channel_message_id: String,
    },

    /// Outbound delivery attempt failed (may be retryable)
    #[error("Delivery failed for {channel}: {reason}")]
    DeliveryFailed {
        /// Target channel
        channel: String,
        /// Failure reason
        reason: String,
        /// Whether this delivery failure can be retried
        retryable: bool,
    },

    /// All retry attempts exhausted, message moved to dead-letter queue
    #[error("Delivery exhausted for {channel} after {attempts} attempts: {reason}")]
    DeliveryExhausted {
        /// Target channel
        channel: String,
        /// Number of delivery attempts made
        attempts: i32,
        /// Last failure reason
        reason: String,
    },

    /// Channel API returned a rate limit response
    #[error("Rate limit exceeded for {channel}: retry after {retry_after_secs}s")]
    RateLimitExceeded {
        /// Channel that is rate-limited
        channel: String,
        /// Seconds to wait before retrying
        retry_after_secs: u64,
    },

    /// Channel is not configured for this tenant
    #[error("Channel {channel} is not configured")]
    ChannelNotConfigured {
        /// Unconfigured channel type
        channel: String,
    },

    /// Invalid webhook payload or message format
    #[error("Invalid payload for {channel}: {reason}")]
    InvalidPayload {
        /// Source channel
        channel: String,
        /// Parse error details
        reason: String,
    },

    /// Messaging session not found
    #[error("Session {session_id} not found")]
    SessionNotFound {
        /// Missing session identifier
        session_id: String,
    },

    /// Messaging session has expired
    #[error("Session {session_id} expired")]
    SessionExpired {
        /// Expired session identifier
        session_id: String,
    },

    /// Media upload to channel failed
    #[error("Media upload failed for {channel}: {reason}")]
    MediaUploadFailed {
        /// Target channel
        channel: String,
        /// Upload failure reason
        reason: String,
    },

    /// Channel API returned a non-retryable error
    #[error("Channel API error for {channel}: HTTP {status_code} - {message}")]
    ChannelApiError {
        /// Target channel
        channel: String,
        /// HTTP status code from channel API
        status_code: u16,
        /// Error message from channel API
        message: String,
    },

    /// Link verification code has expired (past 10-minute TTL)
    #[error("Link code has expired")]
    LinkCodeExpired,

    /// Link verification code has already been consumed
    #[error("Link code has already been used")]
    LinkCodeAlreadyUsed,

    /// Channel identity is already linked to a Pierre user in this tenant
    #[error("Channel {channel} identity {channel_user_id} is already linked")]
    ChannelAlreadyLinked {
        /// Channel type that is already linked
        channel: String,
        /// Channel-specific user identifier
        channel_user_id: String,
    },

    /// Link code cannot be completed (already has a `user_id` set)
    #[error("Link code {code} cannot be completed: {reason}")]
    LinkCodeNotCompletable {
        /// The link code that cannot be completed
        code: String,
        /// Reason the completion failed
        reason: String,
    },

    /// Attempted to unlink a channel that is not linked
    #[error("Channel {channel} is not linked for this user")]
    ChannelNotLinked {
        /// Channel type that is not linked
        channel: String,
    },
}

impl MessagingError {
    /// Whether this error indicates a condition that may succeed if retried
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::DeliveryFailed { retryable, .. } => *retryable,
            Self::RateLimitExceeded { .. } => true,
            Self::ChannelApiError { status_code, .. } => matches!(status_code, 429 | 500..=599),
            _ => false,
        }
    }

    /// Suggested delay before retrying, if applicable
    #[must_use]
    pub const fn retry_after_secs(&self) -> Option<u64> {
        match self {
            Self::RateLimitExceeded {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        }
    }
}

/// Convenience type alias for messaging operations
pub type MessagingResult<T> = Result<T, MessagingError>;

use super::{AppError, ErrorCode};

/// Conversion from `MessagingError` to `AppError`
impl From<MessagingError> for AppError {
    fn from(error: MessagingError) -> Self {
        let code = match &error {
            MessagingError::SignatureVerificationFailed { .. }
            | MessagingError::ReplayDetected { .. } => ErrorCode::AuthInvalid,
            MessagingError::DuplicateMessage { .. }
            | MessagingError::ChannelAlreadyLinked { .. } => ErrorCode::ResourceAlreadyExists,
            MessagingError::RateLimitExceeded { .. } => ErrorCode::ExternalRateLimited,
            MessagingError::ChannelNotConfigured { .. } => ErrorCode::ConfigError,
            MessagingError::InvalidPayload { .. }
            | MessagingError::LinkCodeExpired
            | MessagingError::LinkCodeAlreadyUsed
            | MessagingError::LinkCodeNotCompletable { .. } => ErrorCode::InvalidInput,
            MessagingError::SessionNotFound { .. } | MessagingError::ChannelNotLinked { .. } => {
                ErrorCode::ResourceNotFound
            }
            MessagingError::SessionExpired { .. } => ErrorCode::AuthExpired,
            MessagingError::DeliveryFailed { .. }
            | MessagingError::DeliveryExhausted { .. }
            | MessagingError::MediaUploadFailed { .. }
            | MessagingError::ChannelApiError { .. } => ErrorCode::ExternalServiceError,
        };
        Self::new(code, error.to_string())
    }
}
