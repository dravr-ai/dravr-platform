// ABOUTME: Re-exports repository traits from pierre-database crate
// ABOUTME: Provides direct implementations bridging domain-manager traits to SQLite Database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use crate::repositories::*;

/// The `AgentsRepository` shell on `Database` (`SQLite`), over the shared body
mod agents_impl;
