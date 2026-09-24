// ABOUTME: Universal protocol envelope shared by every tool execution surface
// ABOUTME: Hosts UniversalRequest/Response, AuthService, UniversalExecutor, and provider helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Authentication service used by universal protocol handlers
pub mod auth;
/// Whose session a `TrainingPeaks` read goes through; a coach account's own read is refused in words
mod delegated_auth;
/// `UniversalExecutor` dispatch surface that fans every protocol call onto the tool registry
pub mod executor;
/// Output format helpers (JSON / TOON envelopes) shared across handlers
pub mod format;
/// Shared provider helper functions (tenant-aware credential resolution, provider creation)
pub mod provider_helpers;
/// The one-time push telling a user a provider connection needs reconnecting
mod reauth_notice;
/// Classification of a failed token refresh: a refusal of the grant or client, or transient
mod refresh_failure;
/// Synced sleep and recovery reads, merged across sources (sleep + analytics tools)
pub mod sleep_helpers;
/// Write-back of a pair a provider refreshed on its own, over the row it was read from
mod token_writeback;
/// Core universal protocol types (`UniversalRequest`, `UniversalResponse`, executor alias)
pub mod types;

pub use auth::AuthService;
pub use executor::UniversalExecutor;
pub use types::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalTool,
    UniversalToolExecutor, META_AUTH_REQUIRED_PROVIDER,
};
