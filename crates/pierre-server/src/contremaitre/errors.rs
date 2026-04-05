// ABOUTME: Structured error types for the contremaitre prompt hot-reload system
// ABOUTME: Maps to AppError via ErrorCode for consistent API error responses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::error::Error;
use std::fmt;

use crate::errors::{AppError, ErrorCode};

/// Errors specific to the contremaitre prompt management system.
#[derive(Debug)]
pub enum ContremaitreError {
    /// GitHub API returned an error response
    GitHubApi {
        /// HTTP status code
        status: u16,
        /// Error message from the API
        message: String,
    },
    /// Failed to parse the manifest.json file
    ManifestParse(String),
    /// Content hash does not match expected value
    HashMismatch {
        /// Prompt key that failed verification
        key: String,
        /// SHA-256 hash recorded in the manifest
        expected: String,
        /// SHA-256 hash computed from downloaded content
        actual: String,
    },
    /// Webhook HMAC-SHA256 signature verification failed
    SignatureVerification,
    /// Configuration is missing or invalid
    Config(String),
    /// HTTP client error during GitHub communication
    HttpClient(String),
}

impl fmt::Display for ContremaitreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GitHubApi { status, message } => {
                write!(f, "GitHub API error ({status}): {message}")
            }
            Self::ManifestParse(msg) => write!(f, "manifest parse error: {msg}"),
            Self::HashMismatch {
                key,
                expected,
                actual,
            } => write!(
                f,
                "hash mismatch for {key}: expected {expected}, got {actual}"
            ),
            Self::SignatureVerification => write!(f, "webhook signature verification failed"),
            Self::Config(msg) => write!(f, "contremaitre config error: {msg}"),
            Self::HttpClient(msg) => write!(f, "HTTP client error: {msg}"),
        }
    }
}

impl Error for ContremaitreError {}

impl From<ContremaitreError> for AppError {
    fn from(err: ContremaitreError) -> Self {
        match &err {
            ContremaitreError::GitHubApi { status, .. } if *status == 404 => {
                Self::new(ErrorCode::ResourceNotFound, err.to_string())
            }
            ContremaitreError::GitHubApi { status, .. } if *status == 401 || *status == 403 => {
                Self::new(ErrorCode::ExternalAuthFailed, err.to_string())
            }
            ContremaitreError::GitHubApi { status, .. } if *status == 429 => {
                Self::new(ErrorCode::ExternalRateLimited, err.to_string())
            }
            ContremaitreError::GitHubApi { .. } => {
                Self::new(ErrorCode::ExternalServiceError, err.to_string())
            }
            ContremaitreError::ManifestParse(_) | ContremaitreError::HashMismatch { .. } => {
                Self::new(ErrorCode::InvalidFormat, err.to_string())
            }
            ContremaitreError::SignatureVerification => {
                Self::new(ErrorCode::AuthInvalid, err.to_string())
            }
            ContremaitreError::Config(_) => Self::new(ErrorCode::ConfigError, err.to_string()),
            ContremaitreError::HttpClient(_) => {
                Self::new(ErrorCode::ExternalServiceUnavailable, err.to_string())
            }
        }
    }
}

impl From<reqwest::Error> for ContremaitreError {
    fn from(err: reqwest::Error) -> Self {
        Self::HttpClient(err.to_string())
    }
}
