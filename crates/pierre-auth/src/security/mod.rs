// ABOUTME: Security module for secure HTTP cookies and CSRF request hardening
// ABOUTME: Browser security response headers are set at the nginx edge, not here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Security Module
//!
//! Security features for Pierre MCP Server including:
//! - Secure HTTP cookie utilities
//! - CSRF protection
//!
//! Browser security response headers (`X-Content-Type-Options`,
//! `X-Frame-Options`, `Referrer-Policy`, `Strict-Transport-Security`,
//! `Content-Security-Policy`) are owned by nginx —
//! `docker/images/frontend/security-headers.conf`, included at server level and
//! in every location that declares its own `add_header`. The API service runs
//! behind that nginx with internal-only ingress, so it serves no header set of
//! its own.

/// Secure HTTP cookie utilities
pub mod cookies;
/// CSRF protection token management
pub mod csrf;
