// ABOUTME: Database abstraction layer for Pierre MCP Server
// ABOUTME: Plugin architecture for database support with SQLite and PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Database provider factory
pub mod factory;

/// PostgreSQL database implementation
#[cfg(feature = "postgresql")]
pub mod postgres;

/// Shared database logic (enum conversions, validation, mappers, encryption, etc.)
pub mod shared;

pub use pierre_database::provider::DatabaseProvider;
pub use pierre_database::provider::{
    A2ADbOps, AdminDbOps, ApiKeyDbOps, ChatDbOps, OAuth2ServerOps, OAuthAccountOps, OAuthDbOps,
    OAuthTokenOps, SecurityDbOps, SocialDbOps, TenantDbOps, UsageDbOps, UserDbOps,
};
