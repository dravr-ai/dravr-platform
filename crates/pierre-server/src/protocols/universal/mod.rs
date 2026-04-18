// ABOUTME: Universal protocol module with clean architecture
// ABOUTME: Modular components for tool execution, authentication, and routing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Authentication service for protocol requests
pub mod auth_service;
/// Universal tool executor
pub mod executor;
/// Handler-side bodies reached via `McpTool::execute` delegation
pub mod handlers;
/// Universal protocol types and interfaces
pub mod types;

// Re-export core types
pub use types::{UniversalRequest, UniversalResponse, UniversalTool, UniversalToolExecutor};

// Re-export new architecture components
/// Authentication service for universal protocol
pub use auth_service::AuthService;
/// Main executor for universal protocol tools
pub use executor::UniversalExecutor;
