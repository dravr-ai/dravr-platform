// ABOUTME: Domain-specific database provider traits decomposed from the monolithic god-trait
// ABOUTME: DatabaseProvider compound supertrait aggregates all domain traits for factory dispatch
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

mod a2a;
mod admin;
mod api_key;
mod chat;
mod oauth;
mod security;
mod social;
mod tenant;
mod usage;
mod user;

pub use a2a::A2ADbOps;
pub use admin::AdminDbOps;
pub use api_key::ApiKeyDbOps;
pub use chat::ChatDbOps;
pub use oauth::{OAuth2ServerOps, OAuthAccountOps, OAuthDbOps, OAuthTokenOps};
pub use security::SecurityDbOps;
pub use social::SocialDbOps;
pub use tenant::TenantDbOps;
pub use usage::UsageDbOps;
pub use user::UserDbOps;

use async_trait::async_trait;
use pierre_core::errors::AppResult;

/// Compound supertrait aggregating all domain database operations.
///
/// Implemented by the `Database` factory enum in the main crate.
/// Blanket-implemented for any type satisfying all domain trait bounds.
#[async_trait]
pub trait DatabaseProvider:
    UserDbOps
    + OAuthTokenOps
    + OAuth2ServerOps
    + OAuthAccountOps
    + ApiKeyDbOps
    + UsageDbOps
    + A2ADbOps
    + AdminDbOps
    + TenantDbOps
    + ChatDbOps
    + SecurityDbOps
    + SocialDbOps
    + Send
    + Sync
    + Clone
{
    /// Create a new database connection with encryption key
    async fn new(database_url: &str, encryption_key: Vec<u8>) -> AppResult<Self>
    where
        Self: Sized;

    /// Run database migrations to set up schema
    async fn migrate(&self) -> AppResult<()>;
}
