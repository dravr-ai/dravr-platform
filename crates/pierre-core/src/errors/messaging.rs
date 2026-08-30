// ABOUTME: Re-exports messaging error types from dravr-canot standalone crate
// ABOUTME: Keeps the platform-specific From<MessagingError> for AppError conversion
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// Canonical messaging error types live in dravr-canot
pub use dravr_canot::error::{MessagingError, MessagingResult};

use super::{AppError, ErrorCode};

/// Conversion from `MessagingError` to `AppError` for HTTP status code mapping
impl From<MessagingError> for AppError {
    fn from(error: MessagingError) -> Self {
        let code = match &error {
            MessagingError::SignatureVerificationFailed { .. }
            | MessagingError::ReplayDetected { .. } => ErrorCode::AuthInvalid,
            MessagingError::DuplicateMessage { .. }
            | MessagingError::ChannelAlreadyLinked { .. } => ErrorCode::ResourceAlreadyExists,
            MessagingError::RateLimitExceeded { .. } => ErrorCode::ExternalRateLimited,
            MessagingError::ChannelNotConfigured { .. } => ErrorCode::ConfigError,
            // `OperationNotSupported` sits here rather than with the external
            // failures: the channel has no API for what was asked, so nothing
            // was attempted and no retry will help. That is a request the
            // channel cannot serve, not a service that failed.
            MessagingError::InvalidPayload { .. }
            | MessagingError::LinkCodeExpired
            | MessagingError::LinkCodeAlreadyUsed
            | MessagingError::LinkCodeNotCompletable { .. }
            | MessagingError::OperationNotSupported { .. } => ErrorCode::InvalidInput,
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
