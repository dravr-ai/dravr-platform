// ABOUTME: The SQLite migration set, embedded at compile time and held in static storage
// ABOUTME: Its own module so the array the macro expands to is never a local in the migrate path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where the schema comes from.
//!
//! `sqlx::migrate!` expands to the whole set as one array. Called inline, that
//! array is a local, and this project has enough migrations for such a local
//! to be worth avoiding — so it is bound to a `static`, in static storage, the
//! way the `PostgreSQL` backend embeds its own set.

use sqlx::migrate::Migrator;

/// Every `SQLite` migration in `migrations/`, in order, embedded in the binary
/// so they apply regardless of the process's working directory.
pub static SQLITE_MIGRATIONS: Migrator = sqlx::migrate!("./migrations");
