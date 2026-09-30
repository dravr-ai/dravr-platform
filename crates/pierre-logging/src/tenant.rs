// ABOUTME: Tenant-aware logging utilities for structured, contextual logging
// ABOUTME: Provides logging macros and utilities that automatically include tenant and user context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::TenantId;
use tracing::Span;
use uuid::Uuid;

/// Record tenant context in current span
pub fn record_tenant_context(user_id: Uuid, tenant_id: TenantId, auth_method: &str) {
    let span = Span::current();
    span.record("user_id", user_id.to_string())
        .record("tenant_id", tenant_id.to_string())
        .record("auth_method", auth_method);
}

/// Record request context in current span
pub fn record_request_context(request_id: &str, method: &str, path: &str) {
    let span = Span::current();
    span.record("request_id", request_id)
        .record("http_method", method)
        .record("http_path", path);
}

/// Record performance metrics in current span
pub fn record_performance_metrics(duration_ms: u64, success: bool) {
    let span = Span::current();
    span.record("duration_ms", duration_ms)
        .record("success", success);
}

/// Create a tenant-aware span for operations
#[macro_export]
macro_rules! tenant_span {
    (info, $name:expr, $user_id:expr, $tenant_id:expr) => {
        tracing::info_span!(
            $name,
            user_id = %$user_id,
            tenant_id = %$tenant_id,
            duration_ms = tracing::field::Empty,
            success = tracing::field::Empty,
        )
    };
    (debug, $name:expr, $user_id:expr, $tenant_id:expr) => {
        tracing::debug_span!(
            $name,
            user_id = %$user_id,
            tenant_id = %$tenant_id,
            duration_ms = tracing::field::Empty,
            success = tracing::field::Empty,
        )
    };
}

/// Create a request-aware span for HTTP operations
#[macro_export]
macro_rules! request_span {
    (info, $name:expr, $request_id:expr, $method:expr, $path:expr) => {
        tracing::info_span!(
            $name,
            request_id = %$request_id,
            http_method = %$method,
            http_path = %$path,
            user_id = tracing::field::Empty,
            tenant_id = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
            status_code = tracing::field::Empty,
        )
    };
    (debug, $name:expr, $request_id:expr, $method:expr, $path:expr) => {
        tracing::debug_span!(
            $name,
            request_id = %$request_id,
            http_method = %$method,
            http_path = %$path,
            user_id = tracing::field::Empty,
            tenant_id = tracing::field::Empty,
            duration_ms = tracing::field::Empty,
            status_code = tracing::field::Empty,
        )
    };
}
