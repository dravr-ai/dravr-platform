// ABOUTME: Universal protocol envelope shared by every tool execution surface
// ABOUTME: Hosts UniversalRequest/Response, AuthService, UniversalExecutor, and provider helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Authentication service used by universal protocol handlers
pub mod auth;
/// Whether a human coach writes a coached athlete's coaching-platform calendar, and the writer they write it with
pub(crate) mod coach_calendar;
/// Whether an athlete has any data behind them: a connection, a token or an uploaded file
mod data_source;
/// Whose session a `TrainingPeaks` read goes through; a coach account's own read is refused in words
mod delegated_auth;
/// `UniversalExecutor` dispatch surface that fans every protocol call onto the tool registry
pub mod executor;
/// Output format helpers (JSON / TOON envelopes) shared across handlers
pub mod format;
/// Shared provider helper functions (tenant-aware credential resolution, provider creation)
pub mod provider_helpers;
/// The one reconnect notice a flagged connection owes its athlete, whichever path flagged it
pub mod reauth_notice;
/// Classification of a failed token refresh: a refusal of the grant or client, or transient
mod refresh_failure;
/// A read repeated within one turn, answered with a pointer to its result
mod repeated_read;
/// Synced sleep and recovery reads, merged across sources (sleep + analytics tools)
pub mod sleep_helpers;
/// Write-back of a pair a provider refreshed on its own, over the row it was read from
mod token_writeback;
/// Reads a tool's `ToolResponse` back: raised error codes, sentinels and the universal reply
mod tool_response;
/// Core universal protocol types (`UniversalRequest`, `UniversalResponse`, executor alias)
pub mod types;

pub use auth::AuthService;
pub use coach_calendar::delegation_services;
pub use executor::UniversalExecutor;
pub use types::{
    auth_required_provider, is_no_provider_refusal, UniversalRequest, UniversalResponse,
    UniversalTool, UniversalToolExecutor, META_AUTH_REQUIRED_PROVIDER, META_NO_PROVIDER_CONNECTED,
};
