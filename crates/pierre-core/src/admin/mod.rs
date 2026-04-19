// ABOUTME: Admin authentication and authorization types for pierre-core
// ABOUTME: Tokens, permissions, and audit types used by DatabaseProvider trait
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Admin data models (tokens, permissions, usage)
pub mod models;

/// JWT signing abstraction and admin token management
pub mod jwt;

#[cfg(feature = "admin-jwt")]
pub use jwt::AdminJwtManager;
#[cfg(feature = "admin-jwt")]
pub use jwt::TokenGenerationConfig;
#[cfg(feature = "admin-jwt")]
pub use jwt::TokenScope;
