// ABOUTME: Re-export of tool execution service for backward compatibility
// ABOUTME: Implementation moved to services/tool_execution.rs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tool loop strategies — re-exported from [`crate::services::tool_execution`].
//!
//! This module exists for backward compatibility. New code should import
//! directly from `crate::services::tool_execution`.

pub use crate::services::tool_execution::*;
