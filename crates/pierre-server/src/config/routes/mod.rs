// ABOUTME: HTTP route handlers for configuration management
// ABOUTME: Re-exports the admin configuration route module
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! HTTP routes for configuration management
//!
//! This module provides REST API endpoints for managing admin-configurable
//! parameters.
//!
//! An athlete's own configuration, fitness configuration included, is read
//! and written through the configuration tools, which persist it; there is no
//! REST surface for it.

/// Admin configuration API route handlers
pub mod admin;

// Re-export route handlers for convenience
pub use admin::{admin_config_router, AdminConfigState};
