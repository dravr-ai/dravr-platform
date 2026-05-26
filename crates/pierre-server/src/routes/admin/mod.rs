// ABOUTME: Admin routes module — diagnostics endpoints that depend on the
// ABOUTME: pierre-server-internal ToolRegistry. Other admin routes live in pierre-routes-admin.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin routes that depend on pierre-server internals.
//!
//! The `diagnostics` sub-module needs the pierre-server-internal
//! [`pierre_tool_runtime::registry::ToolRegistry`] for the
//! `/admin/diagnostics/tool-schema-size` endpoint and is mounted by the
//! composition root alongside [`pierre_routes_admin::AdminRoutes::routes`].

pub mod diagnostics;
