// ABOUTME: Database repository trait definitions for Pierre fitness platform
// ABOUTME: Defines focused repository traits and lifecycle-only DatabaseProvider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![warn(missing_docs)]

//! # pierre-database
//!
//! Repository trait definitions for database abstraction. This crate defines
//! the public API surface for all database operations as focused, cohesive
//! trait groups. Concrete implementations (`SQLite`, `PostgreSQL`) live in the
//! main `pierre_mcp_server` crate and implement repository traits directly.
//!
//! [`DatabaseProvider`](repositories::DatabaseProvider) handles lifecycle only
//! (`new` + `migrate`). Data access goes through individual repository traits.

/// Repository trait definitions for each database domain.
pub mod repositories;

pub use repositories::DatabaseProvider;
