// ABOUTME: Data Transfer Objects (DTOs) for authentication HTTP endpoints
// ABOUTME: Request/response shapes consumed by REST, MCP, and A2A entry points
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Data Transfer Objects for authentication endpoints.
//!
//! These types describe the wire shapes for the auth API surface
//! (registration, login, OAuth provider linking, session restore,
//! password reset, locale/persona updates). They live in `pierre-auth`
//! so that services consuming them (`pierre-server::services::auth`,
//! `pierre-server::services::oauth_flow`) can be expressed independently
//! of the route module that wires the HTTP handlers.

/// Authentication request/response DTOs.
pub mod auth;
