// ABOUTME: Database repository trait definitions for Pierre fitness platform
// ABOUTME: Defines 27 focused repository traits decomposing the monolithic DatabaseProvider
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![warn(missing_docs)]

//! # pierre-database
//!
//! Repository trait definitions for database abstraction. This crate defines
//! the public API surface for all database operations as focused, cohesive
//! trait groups. Concrete implementations (`SQLite`, `PostgreSQL`) live in the
//! main `pierre_mcp_server` crate and depend on this crate for the trait contracts.
//!
//! The [`DatabaseProvider`](provider::DatabaseProvider) trait is the internal
//! god-trait that all backends implement. Blanket implementations bridge each
//! focused repository trait to the corresponding `DatabaseProvider` methods.

/// Repository trait definitions for each database domain.
pub mod repositories;

/// Domain-specific database provider traits decomposed from the monolithic god-trait.
pub mod provider;

/// Blanket implementations bridging repository traits to `DatabaseProvider`.
pub mod blanket_impls;
