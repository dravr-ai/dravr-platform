// ABOUTME: Request tracing middleware for correlation and structured logging
// ABOUTME: Generates request IDs and creates spans for all HTTP requests with tenant context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::field::Empty;
use uuid::Uuid;

/// Request context that flows through the entire request lifecycle
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// Unique identifier for this request
    pub request_id: String,
    /// Authenticated user ID (if available)
    pub user_id: Option<Uuid>,
    /// Tenant ID for multi-tenancy (if available)
    pub tenant_id: Option<Uuid>,
    /// Authentication method used (e.g., "Bearer", "`ApiKey`")
    pub auth_method: Option<String>,
}

impl RequestContext {
    /// Create new request context with generated request ID
    #[must_use]
    pub fn new() -> Self {
        Self {
            request_id: format!("req_{}", Uuid::new_v4().simple()),
            user_id: None,
            tenant_id: None, // Populated by auth middleware via with_auth() after authentication
            auth_method: None,
        }
    }

    /// Update context with authentication information
    #[must_use]
    pub fn with_auth(
        mut self,
        user_id: Uuid,
        tenant_id: Option<Uuid>,
        auth_method: String,
    ) -> Self {
        self.user_id = Some(user_id);
        self.tenant_id = tenant_id;
        self.auth_method = Some(auth_method);
        self
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a tracing span for HTTP requests
pub fn create_request_span(method: &str, path: &str) -> tracing::Span {
    tracing::info_span!(
        "http_request",
        method = %method,
        path = %path,
        request_id = Empty,
        user_id = Empty,
        tenant_id = Empty,
        auth_method = Empty,
        status_code = Empty,
        duration_ms = Empty,
    )
}

/// Create a tracing span for MCP operations
pub fn create_mcp_span(operation: &str) -> tracing::Span {
    tracing::info_span!(
        "mcp_operation",
        operation = %operation,
        request_id = Empty,
        user_id = Empty,
        tenant_id = Empty,
        tool_name = Empty,
        duration_ms = Empty,
        success = Empty,
    )
}

/// Create a tracing span for database operations
pub fn create_database_span(operation: &str, table: &str) -> tracing::Span {
    tracing::debug_span!(
        "database_operation",
        operation = %operation,
        table = %table,
        request_id = Empty,
        user_id = Empty,
        tenant_id = Empty,
        duration_ms = Empty,
        rows_affected = Empty,
    )
}
