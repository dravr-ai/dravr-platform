// ABOUTME: Why a plan push failed to write an entry, as a stable code a client words
// ABOUTME: Read from the provider error's code at the failure site, never from its text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, ErrorCode};

/// Why an entry's write failed, for a client that words it (carnet#1841).
///
/// It stays off the wire and out of the tool's schema: the entry's `error`
/// text is for the agent and the log, this is for the athlete's screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FailureCause {
    /// The provider answered the write with a refusal.
    ProviderRefused,
    /// The provider could not be reached, or asked to be tried later.
    ProviderUnreachable,
    /// Dravr's own record of the entry could not be written.
    #[default]
    Internal,
}

impl FailureCause {
    /// The cause of a provider call that failed with `error`.
    pub(super) const fn of_provider(error: &AppError) -> Self {
        match error.code {
            ErrorCode::ExternalServiceUnavailable
            | ErrorCode::ExternalRateLimited
            | ErrorCode::ResourceUnavailable => Self::ProviderUnreachable,
            _ => Self::ProviderRefused,
        }
    }

    /// The stable code a client branches on.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderRefused => "provider_refused",
            Self::ProviderUnreachable => "provider_unreachable",
            Self::Internal => "internal",
        }
    }
}
