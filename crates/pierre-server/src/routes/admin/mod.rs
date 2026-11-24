// ABOUTME: Admin routes module — diagnostics endpoints that depend on the
// ABOUTME: pierre-server-internal ToolRuntime. Other admin routes live in pierre-routes-admin.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin routes that depend on pierre-server internals.
//!
//! The `diagnostics` sub-module runs the capture refresh on the
//! composition root's [`pierre_tool_runtime::runtime::ToolRuntime`] and is
//! mounted alongside [`pierre_routes_admin::AdminRoutes::routes`].

pub mod diagnostics;
