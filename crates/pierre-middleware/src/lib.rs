// ABOUTME: HTTP middleware for request tracing, authentication, and context propagation
// ABOUTME: Provides request ID generation, span creation, and tenant context for structured logging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Middleware
//!
//! Axum-based HTTP middleware shared across every Pierre transport (REST, MCP,
//! A2A, messaging webhooks). Auth (JWT + API key + cookie), tenant resolution,
//! CSRF, CORS, rate limiting, redaction, tracing, request-id propagation, and
//! admin authorization guards.
//!
//! Middleware functions are generic over `pierre_runtime_context::MiddlewareCtx`
//! so the crate stays decoupled from pierre-server's composition root; the
//! server impls the trait on its `ServerContext` and passes it as Axum state.

/// Admin authorization guard for routes requiring admin privileges
pub mod admin_guard;
/// Authentication middleware for MCP and API requests
pub mod auth;
/// CORS middleware configuration
pub mod cors;
/// CSRF validation middleware
pub mod csrf;
/// Axum extractors for common authentication patterns
pub mod extractors;
/// Short-lived signed link-tokens for channel-initiated provider connection flows
pub mod provider_link_token;
/// Rate limiting middleware and utilities
pub mod rate_limiting;
/// PII redaction and sensitive data masking
pub mod redaction;
/// Request ID generation and propagation
pub mod request_id;
/// Single HTTP failure logger — endpoint-aware, backpressure-aware
pub mod response_failure_log;
/// HTTP request metrics + inbound W3C trace-context extraction (OTLP telemetry)
#[cfg(feature = "telemetry")]
pub mod telemetry;
/// Axum extractor for a tenant id supplied in the URL path
pub mod tenant_path;

/// Tenant context extraction middleware
pub mod tenant;
/// Request tracing and context propagation
pub mod tracing;

// Admin authorization guard

/// Cookie/session-auth + `is_admin` middleware for `/api/admin/...` routes
pub use admin_guard::cookie_admin_middleware;
/// Build the role-derived `ValidatedAdminToken` for a cookie-authenticated admin
pub use admin_guard::cookie_admin_token;
/// Require admin privileges for a user
pub use admin_guard::require_admin;

// Authentication middleware

/// MCP authentication middleware
pub use auth::McpAuthMiddleware;
/// CSRF protection layer for Axum router (validates cookie-auth state-changing requests)
pub use csrf::csrf_protection_layer;
/// Pure-function CSRF validator (header extraction + method-class check + token verification)
pub use csrf::validate_csrf_token;
/// Shared auth extraction from headers (for handlers that need custom post-processing)
pub use extractors::extract_auth_from_headers;
/// Axum extractor for authenticated user context from JWT or cookie
pub use extractors::AuthenticatedUser;

// CORS middleware

/// Setup CORS layer for HTTP endpoints
pub use cors::setup_cors;

// Rate limiting middleware and utilities

/// Check rate limit and send error response
pub use rate_limiting::check_rate_limit_and_respond;
/// Create rate limit error
pub use rate_limiting::create_rate_limit_error;
/// Create rate limit headers
pub use rate_limiting::create_rate_limit_headers;
/// Rate limit headers module
pub use rate_limiting::headers;

// PII-safe logging and redaction

/// Mask email addresses for logging
pub use redaction::mask_email;
/// Redact sensitive HTTP headers
pub use redaction::redact_headers;
/// Redact JSON fields by pattern
pub use redaction::redact_json_fields;
/// Redact session IDs for safe logging
pub use redaction::redact_session_id;
/// Redact token patterns from strings
pub use redaction::redact_token_patterns;
/// Bounded tenant label for tracing
pub use redaction::BoundedTenantLabel;
/// Bounded user label for tracing
pub use redaction::BoundedUserLabel;
/// Redaction configuration
pub use redaction::RedactionConfig;
/// Redaction features toggle
pub use redaction::RedactionFeatures;

// Request ID middleware

/// Request ID middleware function
pub use request_id::request_id_middleware;
/// Request ID extractor
pub use request_id::RequestId;
/// Endpoint-aware, backpressure-aware HTTP failure logging middleware
pub use response_failure_log::response_failure_log_middleware;
#[cfg(feature = "telemetry")]
pub use telemetry::telemetry_middleware;

// Request tracing and context management

/// Create database operation span
pub use tracing::create_database_span;
/// Create MCP operation span
pub use tracing::create_mcp_span;
/// Create HTTP request span
pub use tracing::create_request_span;
/// Request context for tracing
pub use tracing::RequestContext;

// Tenant context middleware

/// Require tenant context helper function
pub use tenant::require_tenant_context;
/// Tenant context extraction middleware
pub use tenant::tenant_context_middleware;
/// Extracted tenant context wrapper for request extensions
pub use tenant::ExtractedTenantContext;
