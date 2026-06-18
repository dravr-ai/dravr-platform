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

/// Data transfer types for seeder binaries
pub mod seed_models;

/// Groups `UserFact`s into the Dossier's pillar / north-star / medical buckets.
mod dossier_facts;

/// `SQLite` database implementation (core database operations)
pub mod database;

/// Database backend plugins (factory, shared utilities, `PostgreSQL`)
pub mod backends;

/// Trait-object registry for repository access without runtime enum dispatch
pub mod repository_registry;

/// Repository view-structs — narrow projections of `RepositoryRegistry` for
/// consumers that only need a cohesive subset of repository traits.
pub mod views;

pub use repositories::DatabaseProvider;
pub use repository_registry::RepositoryRegistry;
pub use views::{AuthRepos, CoachRepos, ContentRepos, FitnessRepos, SocialRepos, UsageRepos};

// The continuous time-series store is riviere's `TimeSeriesStore`. Re-export the
// trait and its data types so consumers of the `time_series_points` registry
// field can drive it without taking their own dravr-riviere dependency.
pub use dravr_riviere::{
    AggregatedPoint, Aggregation, DataPoint, QueryResult, SeriesType, TimeRange, TimeSeriesStore,
};
