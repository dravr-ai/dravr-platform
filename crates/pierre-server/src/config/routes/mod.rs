// ABOUTME: HTTP route handlers for configuration management
// ABOUTME: Re-exports the admin and fitness configuration route modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! HTTP routes for configuration management
//!
//! This module provides REST API endpoints for managing:
//! - Admin-configurable parameters
//! - Fitness-specific configurations with tenant isolation
//!
//! An athlete's own configuration is read and written through the
//! `get_user_configuration` / `update_user_configuration` tools, which persist
//! it; there is no REST surface for it.

/// Admin configuration API route handlers
pub mod admin;
/// Fitness-specific configuration route handlers with tenant isolation
pub mod fitness;

// Re-export route handlers for convenience
pub use admin::{admin_config_router, AdminConfigState};
pub use fitness::FitnessConfigurationRoutes;
