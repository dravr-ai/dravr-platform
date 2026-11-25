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
//! - [`activity_fields`] — whether a mapped activity carries a field the
//!   field dictionary names, for the guards that check each mapper.
//! - [`db`] — the single factory every test opens a database through
//!   ([`db::create_test_db`] and friends), honouring `DATABASE_URL`.
//! - [`delegation`] — the group, agent and membership a coach's delegated
//!   link rests on.
//! - [`garmin`] — an athlete's Garmin session and the API grant that pushes
//!   their wellness.
//! - `group_tier` (feature `groups`) — a plan whose tier includes no group
//!   coaching, which no shipping tier is.
//! - [`log_capture`] — the one way a test installs a tracing subscriber and
//!   reads back what the code under test logged, without losing a line to
//!   another test's cached log-site interest.
//! - [`numbers`] — [`numbers::mentions_number`], whether serialized text
//!   carries a number of its own rather than digits inside a timestamp or id.
//! - `providers` (feature `providers`) — the provider registry a test builds,
//!   with every endpoint the server calls at the refused origin.
//! - [`server`] — `User` fixtures in a consistent default shape.
//! - `sse` (feature `sse`) — the frames a client reads off a streaming chat
//!   turn's body.
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
//!
//! `groups` builds the group-tier fixture, and with it `pierre-groups`; only a
//! consumer whose suites drive group creation turns it on.
//!
//! `sse` builds the SSE frame reader, and with it an HTTP client; only a
//! consumer whose suites read a streaming turn turns it on.
//!
//! `providers` builds the offline provider registry, and with it the
//! providers; a consumer whose tests build a `ProviderRegistry` turns it on.
//! `pierre-providers` lists this crate as a dev-dependency, so, as with
//! `pierre-database`, only its integration tests can use the registry: its
//! in-module unit tests build a different `ProviderRegistry` type.

/// Deleting an account the way production does.
pub mod account;
/// Whether a mapped activity carries a field or stream channel, by dictionary name.
pub mod activity_fields;
/// The test-database factory: `SQLite` image clones, `PostgreSQL` template clones.
pub mod db;
/// Coaching-group rows a coach's delegated link rests on.
pub mod delegation;
/// A loopback TCP relay that injects network faults: refuse, blackhole, latency, sever.
pub mod fault_proxy;
/// Small, valid FIT activity files written for tests.
pub mod fit;
/// A Garmin athlete's usr/pwd session and the API grant pushing their wellness.
pub mod garmin;
/// A group tier whose plan includes no group coaching.
#[cfg(feature = "groups")]
pub mod group_tier;
/// Capture one tracing target's events, to assert the lines code logs.
pub mod log_capture;
/// Whole-number search over serialized replies, immune to digits inside timestamps and ids.
pub mod numbers;
/// The provider registry a test builds, every server-called endpoint refused.
#[cfg(feature = "providers")]
pub mod providers;
/// `User` fixtures in a consistent default shape.
pub mod server;
/// The frames a client reads off a streaming chat turn's SSE body.
#[cfg(feature = "sse")]
pub mod sse;
