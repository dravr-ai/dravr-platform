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

/// `PostgreSQL` social features database operations
#[cfg(feature = "postgresql")]
pub mod social_postgres;

/// Shared database logic (enum conversions, validation, mappers, encryption, etc.)
pub mod shared;

pub use crate::repositories::{
    A2ARepository, AdminRepository, ApiKeyRepository, ChatRepository, CoachesRepository,
    FitnessConfigRepository, ImpersonationRepository, InsightRepository, LlmCredentialRepository,
    LlmUsageRepository, MobilityRepository, NotificationRepository, OAuth2ServerRepository,
    OAuthClientStateRepository, OAuthTokenRepository, PasswordResetRepository, ProfileRepository,
    ProviderConnectionRepository, RecipeRepository, SecurityRepository, SocialRepository,
    TenantRepository, ToolSelectionRepository, UsageCounterRepository, UsageRepository,
    UserMcpTokenRepository, UserRepository,
};
pub use crate::DatabaseProvider;
