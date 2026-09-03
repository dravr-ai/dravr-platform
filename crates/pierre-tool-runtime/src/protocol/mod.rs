// ABOUTME: Universal protocol envelope shared by every tool execution surface
// ABOUTME: Hosts UniversalRequest/Response, AuthService, UniversalExecutor, and provider helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Authentication service used by universal protocol handlers
pub mod auth;
/// `UniversalExecutor` dispatch surface that fans every protocol call onto the tool registry
pub mod executor;
/// Output format helpers (JSON / TOON envelopes) shared across handlers
pub mod format;
/// Shared provider helper functions (tenant-aware credential resolution, provider creation)
pub mod provider_helpers;
/// Provider-agnostic sleep fetch + conversion helpers (reused by analytics + sleep tools)
pub mod sleep_helpers;
/// Core universal protocol types (`UniversalRequest`, `UniversalResponse`, executor alias)
pub mod types;

pub use auth::AuthService;
pub use executor::UniversalExecutor;
pub use types::{
    auth_required_provider, UniversalRequest, UniversalResponse, UniversalTool,
    UniversalToolExecutor, META_AUTH_REQUIRED_PROVIDER,
};
