// ABOUTME: Re-export of tool execution service from the services layer
// ABOUTME: Business logic lives in services/tool_execution.rs; this module preserves the public path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool loop strategies — re-exported from [`crate::services::tool_execution`].
//!
//! Business logic lives in the services layer. This module re-exports
//! the public API so existing callers continue to compile.

pub use crate::services::tool_execution::*;
