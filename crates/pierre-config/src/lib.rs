// ABOUTME: Configuration types and loaders for the Pierre platform extracted from pierre-server
// ABOUTME: Owns ServerConfig + sibling configs (network, security, cache, database, MCP, logging, runtime, catalog, validation)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![warn(missing_docs)]

//! # Pierre Config
//!
//! Centralized configuration data types and environment loaders for the Pierre
//! platform. Extracted from `pierre-server` as a workspace leaf crate so the
//! orchestrator (`ServerConfig`) and its sibling configs (network, security,
//! cache, database, MCP, logging, runtime catalog, validation) can be reused
//! by other crates without pulling the full server.
//!
//! Re-exported back into `pierre_mcp_server::config::*` via shim modules so
//! existing call sites and integration tests keep their import paths.

/// Admin configuration parameter catalog (per-category builders + `ParameterDefinition`)
pub mod admin_definitions;
/// Admin configuration data types (parameters, categories, audit, request/response)
pub mod admin_types;
/// External API provider configuration (Strava, Fitbit, Garmin APIs)
pub mod api_providers;
/// Cache and rate limiting configuration (Redis, TTLs, rate limits)
pub mod cache;
/// Configuration parameter catalog and schema definitions
pub mod catalog;
/// Application constants and runtime server config holder (`SERVER_CONFIG` `OnceLock`)
pub mod constants;
/// Database configuration (`DatabaseUrl`, pools, backups, `SQLx`)
pub mod database;
/// Environment and server configuration orchestrator
pub mod environment;
/// Goal management configuration
pub mod goal_management;
/// Logging and PII redaction configuration
pub mod logging;
/// MCP protocol and runtime configuration
pub mod mcp;
/// Network configuration (HTTP client, SSE, CORS, TLS, timeouts)
pub mod network;
/// Runtime configuration management with session-scoped overrides
pub mod runtime;
/// Security configuration (auth, headers, monitoring)
pub mod security;
/// Sleep tool operational parameters (activity limits, trend thresholds)
pub mod sleep_tool_params;
/// Social insights configuration for coach-mediated sharing
pub mod social;
/// Tool selection configuration for global tool disabling via environment variables
pub mod tool_selection;
/// Core configuration type definitions (`LogLevel`, `Environment`, `LlmProviderType`)
pub mod types;
/// Server-level utilities consuming pierre-config types (HTTP client + route timeouts)
pub mod utils;
/// Configuration validation and safety checks
pub mod validation;
