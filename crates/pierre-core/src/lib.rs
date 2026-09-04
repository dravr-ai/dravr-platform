// ABOUTME: Core types and constants for Pierre fitness intelligence platform
// ABOUTME: Foundation crate with error handling, pagination, and constants
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

/// Unified error handling system with standard error codes and HTTP responses
pub mod errors;

/// Application constants and configuration values organized by domain
pub mod constants;

/// Cursor-based pagination for efficient data traversal
pub mod pagination;

/// Core data models (Activity, User, SportType, OAuth, etc.)
pub mod models;

/// Role-based permission system with bitflags
pub mod permissions;

/// Fitness-specific configuration (sport types, zones, thresholds)
pub mod config;

/// Intelligence types (`MaxHrAlgorithm`, fitness profiles)
pub mod intelligence;

/// URL redaction utility for safe logging of connection strings
pub mod redaction;

/// Phase C input sanitization — prompt injection detection for inbound user messages
pub mod safety;

/// Phase C system-prompt fingerprinting for prompt exfiltration defense
pub mod prompt_fingerprint;

/// Reply-side internal-narration scrub (hidden-block/raw-XML meta-commentary)
pub mod narration;

/// Admin authentication and authorization types
pub mod admin;

/// UUID parsing, formatting, and generation utilities
pub mod uuid_utils;

/// Runtime feature flag registry (known keys + compile-time defaults)
pub mod feature_flags;

/// Character-based LLM token estimation (single source of truth for the ~4 chars/token heuristic)
pub mod tokens;

/// Deserializers that accept a whole-valued float where a schema declares an integer
pub mod serde_num;

/// Plain-text markdown stripper for messaging output
pub mod markdown;

/// Sentence-boundary chunking of an over-limit reply into ordered messages
pub mod chunking;

/// HTML escaping utilities for XSS prevention in server-rendered templates
pub mod html;

/// Neutralizing untrusted text before it is interpolated into a structured destination
pub mod untrusted;

/// Authorization header parsing helpers (bearer token extraction, API key detection)
pub mod auth_header;

/// Small constructor helpers for common `AppError` patterns
pub mod error_helpers;

/// Three-way update intent (`keep` / `clear` / `set`) for PATCH-style request bodies
pub mod field_update;

/// LLM provider trait and shared types for pluggable AI model integration
#[cfg(feature = "llm")]
pub mod llm;

/// Pluggable billing provider trait + value types — concrete impls
/// (Stripe, RevenueCat, …) live in their own dravr-* repos.
#[cfg(feature = "billing")]
pub mod billing;

/// Shared HTTP client singletons with connection pooling for outbound requests
#[cfg(feature = "http-client")]
pub mod http_client;

/// Outbound W3C trace-context propagation middleware for the shared HTTP clients
#[cfg(feature = "telemetry")]
pub mod trace_propagation;

/// The athlete's civil clock — local-zone rendering and localized weekday names
pub mod civil_time;
