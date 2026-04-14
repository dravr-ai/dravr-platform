// ABOUTME: Core types and constants for Pierre fitness intelligence platform
// ABOUTME: Foundation crate with error handling, pagination, formatters, and constants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Core
//!
//! Foundation crate providing shared types and constants for the Pierre fitness
//! intelligence platform. This crate is designed to change infrequently, enabling
//! incremental compilation benefits in the workspace.
//!
//! ## Modules
//!
//! - **errors**: Unified error handling with `AppError`, `ErrorCode`, and domain-specific errors
//! - **constants**: Application-wide constants organized by domain
//! - **pagination**: Cursor-based pagination for efficient data traversal
//! - **formatters**: Output format abstraction (JSON, TOON) for LLM-optimized serialization

/// Unified error handling system with standard error codes and HTTP responses
pub mod errors;

/// Application constants and configuration values organized by domain
pub mod constants;

/// Cursor-based pagination for efficient data traversal
pub mod pagination;

/// Output format abstraction (JSON, TOON) for efficient LLM serialization
pub mod formatters;

/// Core data models (Activity, User, SportType, OAuth, etc.)
pub mod models;

/// Role-based permission system with bitflags
pub mod permissions;

/// Fitness-specific configuration (sport types, zones, thresholds)
pub mod config;

/// Intelligence types (`MaxHrAlgorithm`, `InsightSharingPolicy`)
pub mod intelligence;

/// URL redaction utility for safe logging of connection strings
pub mod redaction;

/// Phase C input sanitization — prompt injection detection for inbound user messages
pub mod safety;

/// Phase C system-prompt fingerprinting for prompt exfiltration defense
pub mod prompt_fingerprint;

/// Admin authentication and authorization types
pub mod admin;

/// UUID parsing, formatting, and generation utilities
pub mod uuid_utils;

/// Character-based LLM token estimation (single source of truth for the ~4 chars/token heuristic)
pub mod tokens;

/// LLM provider trait and shared types for pluggable AI model integration
#[cfg(feature = "llm")]
pub mod llm;

/// Shared HTTP client singletons with connection pooling for outbound requests
#[cfg(feature = "http-client")]
pub mod http_client;
