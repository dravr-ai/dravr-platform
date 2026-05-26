// ABOUTME: pierre-server-local tool implementations (endurance workflow tools)
// ABOUTME: Cross-cutting categories live in pierre-tool-runtime::implementations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Tool Implementations (pierre-server local)
//!
//! Most tool implementations live in `pierre_tool_runtime::implementations`.
//! Only the endurance Phase 1/2/3 tools stay here because they touch
//! pierre-server-internal services that have not been extracted yet.

/// Endurance Phase 1 export tools: `export_latest_snapshot`, `export_dossier`
#[cfg(feature = "tools-data")]
pub mod endurance_export;

/// Endurance Phase 2 history tools: `compute_training_history`, `get_training_history`
#[cfg(feature = "tools-data")]
pub mod endurance_history;

/// Endurance Phase 3 intervals/routes tools: `export_intervals`, `export_routes`,
/// `extract_activity_streams`
#[cfg(feature = "tools-data")]
pub mod endurance_intervals;
