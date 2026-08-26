// ABOUTME: Route module organization for Pierre MCP Server HTTP endpoints
// ABOUTME: Provides centralized route definitions organized by domain with clean separation of concerns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Route module for Pierre MCP Server
//!
//! This module organizes all HTTP routes by domain for better maintainability
//! and clear separation of concerns. Each domain module contains only route
//! definitions and thin handler functions that delegate to service layers.
//!
//! Routes are conditionally compiled based on feature flags to support
//! modular server configurations.
//!
//! Many route surfaces have been extracted to dedicated sub-crates:
//! - `pierre-routes-a2a`, `pierre-routes-auth`, `pierre-routes-admin`,
//!   `pierre-routes-coaches`, `pierre-routes-dashboard`,
//!   `pierre-routes-identity`, `pierre-routes-social`,
//!   `pierre-routes-web-admin`
//!
//! The composition root in `mcp::multitenant` mounts those routers via
//! `pierre_routes_<area>::<RouterType>::routes(…)`. Modules in this
//! directory are pierre-server-local (e.g. `mcp`, `health`, `chat`,
//! `messaging`, `webhooks`, `endurance`, `onboarding`).

// ═══════════════════════════════════════════════════════════════
// ALWAYS ENABLED - Core infrastructure
// ═══════════════════════════════════════════════════════════════

/// Health check and system status routes
pub mod health;

// ═══════════════════════════════════════════════════════════════
// PROTOCOL FEATURES
// ═══════════════════════════════════════════════════════════════

/// Model Context Protocol (MCP) server routes
#[cfg(feature = "protocol-mcp")]
pub mod mcp;

// ═══════════════════════════════════════════════════════════════
// CLIENT-WEB FEATURES
// ═══════════════════════════════════════════════════════════════

/// Configuration management routes
#[cfg(feature = "client-settings")]
pub mod configuration;

/// Fitness configuration routes
#[cfg(feature = "client-settings")]
pub mod fitness;

/// Health data persistence routes (sleep, recovery, snapshots, data sources)
#[cfg(feature = "client-settings")]
pub mod health_data;

/// Chat conversation routes for AI assistants
#[cfg(feature = "client-chat")]
pub mod chat;

/// Per-caller slash-command catalogue (`GET /api/commands`).
///
/// Needs the messaging feature: the command registry, its argument
/// signatures and the handlers that answer `is_available` are all loaded
/// under it. Without them there is no catalogue to resolve.
#[cfg(feature = "client-messaging")]
pub mod commands;

/// Usage quota status routes
#[cfg(feature = "client-chat")]
pub mod usage;

/// User-facing harness memory routes (list / forget user_facts)
pub mod memory;

// ═══════════════════════════════════════════════════════════════
// CLIENT-ADMIN FEATURES
// ═══════════════════════════════════════════════════════════════

/// Admin API routes for user management and configuration
#[cfg(feature = "client-admin-api")]
pub mod admin;

/// Onboarding state — self-read endpoint that tells the frontend whether
/// the user still needs to connect a fitness provider.
pub mod oauth_grants;

pub mod onboarding;

/// API key management routes
#[cfg(feature = "client-api-keys")]
pub mod api_keys;

/// Tenant management routes
#[cfg(feature = "client-tenants")]
pub mod tenants;

// ═══════════════════════════════════════════════════════════════
// OTHER CLIENT FEATURES
// ═══════════════════════════════════════════════════════════════

/// User MCP token management routes for AI client authentication
#[cfg(feature = "client-mcp-tokens")]
pub mod user_mcp_tokens;

/// Multi-channel messaging gateway routes (webhook ingress and channel config)
#[cfg(feature = "client-messaging")]
pub mod messaging;

/// GitHub webhook handler for the contremaitre source repository (push events).
pub mod contremaitre_webhook;

/// Endurance Phase 1 read-side endpoints (`GET /api/v1/endurance/{latest,dossier}`).
pub mod endurance;

/// User profile self-service routes (`/api/users/me/*`).
///
/// Currently houses the `timezone` setter that web + mobile clients
/// call after login so the chat prompt can resolve `{{CURRENT_DATE}}`
/// to the user's local calendar day. Kept separate from billing so
/// neither module accumulates unrelated endpoints.
pub mod user_profile;

/// Surface-capability catalogue (`GET /api/surfaces/capabilities`).
///
/// Needs both features: the table is only complete when the chat pipeline is
/// compiled in to resolve it and every messaging channel is compiled in to
/// report its transport. A partial catalogue would generate client constants
/// that quietly omit surfaces.
#[cfg(all(feature = "client-chat", feature = "client-messaging"))]
pub mod surfaces;

/// Chart images for messaging channels: signed short-TTL PNG URLs.
pub mod viz;
/// Provider-pushed health-data webhook routes (WHOOP, Garmin, Oura).
#[cfg(feature = "health-sync")]
pub mod webhooks;
