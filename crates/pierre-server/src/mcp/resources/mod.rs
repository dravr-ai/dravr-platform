// ABOUTME: Facade for the ServerContext dependency-injection container split across this directory
// ABOUTME: Re-exports the public surface (ServerContext, ServerContextOptions, ServerContextBuilder) for crate::mcp::resources::*
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

mod builder;
mod context;
#[cfg(feature = "contremaitre")]
mod contremaitre;
mod lifecycle;
mod options;
mod runtime_traits;
mod slices;
mod tool_runtime;

pub use builder::ServerContextBuilder;
pub use context::ServerContext;
pub use options::ServerContextOptions;
pub use slices::{
    A2ASlice, AuthSlice, BillingSlice, CoachSlice, CommonSlice, FitnessSlice, McpSlice, SseSlice,
};
