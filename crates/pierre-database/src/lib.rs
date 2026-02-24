// ABOUTME: Database abstraction layer with repository traits and implementations
// ABOUTME: Provides SQLite and PostgreSQL backends with focused repository traits
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # pierre-database
//!
//! Database abstraction layer for Pierre fitness platform. This crate provides:
//!
//! - **Repository trait definitions** for each database domain (focused, cohesive trait groups)
//! - **`SQLite` implementation** (default backend)
//! - **`PostgreSQL` implementation** (optional, via `postgresql` feature)
//! - **Shared utilities** for encryption, validation, transactions, and type mapping
//!
//! [`DatabaseProvider`](repositories::DatabaseProvider) handles lifecycle only
//! (`new` + `migrate`). Data access goes through individual repository traits.

/// Repository trait definitions for each database domain.
pub mod repositories;

/// `SQLite` database implementation (core database operations)
pub mod database;

/// Database backend plugins (factory, shared utilities, `PostgreSQL`)
pub mod plugins;

pub use repositories::DatabaseProvider;
