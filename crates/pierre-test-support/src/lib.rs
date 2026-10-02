// ABOUTME: Test fixtures for the workspace's integration tests, kept out of every shipped crate's src/
// ABOUTME: Holds the test-database factory (`db`), user fixtures (`server`) and coaching-group rows (`delegation`); dev-dependency only
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # pierre-test-support
//!
//! Fixtures the integration tests share, moved out of production code by
//! ADR-027: no `src/` of a shipped crate exports test helpers.
//!
//! - [`db`] — the single factory every test opens a database through
//!   ([`db::create_test_db`] and friends), honouring `DATABASE_URL`.
//! - [`delegation`] — the group, agent and membership a coach's delegated
//!   link rests on.
//! - [`server`] — `User` fixtures in a consistent default shape.
//!
//! ## Always a dev-dependency
//!
//! Every consumer lists this crate under `[dev-dependencies]` only. It depends
//! on `pierre-database` as a normal dependency, so `pierre-database` listing it
//! as a dev-dependency forms a cycle that Cargo permits for **integration
//! tests** (`tests/`, `benches/`, `examples/`): those link the one
//! `pierre-database` library this crate was built against.
//!
//! The cycle does **not** work for `pierre-database`'s own in-crate
//! in-module unit tests. That harness compiles `pierre-database` a second
//! time, as the test crate, while this crate still links the ordinary library,
//! so the `Database` it returns is a different type from the one the unit test
//! names and the two never unify. A unit test inside `pierre-database` that
//! needs a real database opens it with the crate's own constructors instead.
//!
//! ## Features
//!
//! `postgresql` turns on the `PostgreSQL` template-clone path. A consumer whose
//! own `postgresql` feature selects that backend forwards it here, so the
//! factory honours a `PostgreSQL` `DATABASE_URL` exactly when the test binary
//! was built able to open one.

/// The test-database factory: `SQLite` image clones, `PostgreSQL` template clones.
pub mod db;
/// Coaching-group rows a coach's delegated link rests on.
pub mod delegation;
/// `User` fixtures in a consistent default shape.
pub mod server;
