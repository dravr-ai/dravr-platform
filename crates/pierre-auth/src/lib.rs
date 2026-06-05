// ABOUTME: Authentication, authorization, and security layer for Pierre platform
// ABOUTME: JWT auth, OAuth2 client/server, tenant management, cryptographic utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authentication, authorization, and security layer for Pierre platform.
//!
//! This crate provides:
//! - JWT-based user authentication and session management
//! - `OAuth2` client (Pierre as client to fitness providers)
//! - `OAuth2` authorization server (Pierre as provider for MCP clients)
//! - API key management for B2B authentication
//! - Multi-tenant data isolation with `OAuth`/LLM credential management
//! - Cryptographic utilities, key management, and security headers

#![deny(unsafe_code)]

/// Auth-related configuration types (OAuth providers, rate limiting)
pub mod config;

/// HTTP client utilities for OAuth flows
pub mod http;

/// Admin authentication (JWKS manager)
pub mod admin;

/// API key management for B2B authentication
pub mod api_keys;

/// JWT-based user authentication and session management
pub mod auth;

/// Cryptographic utilities and key management
pub mod crypto;

/// Data Transfer Objects (request/response shapes) for auth HTTP endpoints
pub mod dto;

/// Firebase Authentication ID-token validation (social login providers via Firebase)
pub mod firebase;

/// Two-tier key management system
pub mod key_management;

/// OAuth 2.0 client (Pierre as client to fitness providers)
pub mod oauth2_client;

/// OAuth 2.0 authorization server (Pierre as provider for MCP clients)
pub mod oauth2_server;

/// Unified rate limiting system
pub mod rate_limiting;

/// Security headers, secure cookies, CSRF protection
pub mod security;

/// Multi-tenant data isolation and OAuth/LLM management
pub mod tenant;

/// Account-status policy gate (`Pending`/`Suspended`/`Active`)
pub mod user_status;
