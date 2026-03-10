// ABOUTME: Domain service layer for business logic extracted from route handlers
// ABOUTME: Provides protocol-agnostic services reusable across REST, MCP, and A2A
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Domain service layer
//!
//! This module contains protocol-agnostic business logic extracted from route handlers.
//! Services are designed to be reusable across REST, MCP, and A2A protocols, ensuring
//! consistent business rules regardless of the entry point.

/// Coach lifecycle operations: prerequisites, assignments, and generation
pub mod coaches;

/// OAuth flow orchestration: state validation, redirect URL parsing
pub mod oauth_flow;

/// Recipe import/export and markdown conversion
pub mod recipes;

/// Tenant administration: slug validation, tenant creation, user provisioning
pub mod tenant_admin;

/// Chat orchestration: conversation creation, message persistence, LLM dispatch coordination
pub mod chat_orchestration;

/// Social insights: friend-request validation, user search enrichment, insight adaptation
pub mod social_insights;

/// Usage counter service: quota enforcement with burst zones and warning thresholds
pub mod usage_counter;

/// Background task for periodic pruning of old usage counter records
pub mod usage_pruning;

/// Background outbound retry worker for messaging delivery queue
#[cfg(feature = "client-messaging")]
pub mod messaging_outbound;

/// Expo Push Service client for sending push notifications via Expo's HTTP API
#[cfg(feature = "client-notifications")]
pub mod expo_push;

/// Central notification dispatch pipeline with preference checking and push delivery
#[cfg(feature = "client-notifications")]
pub mod notification_dispatch;

/// Intelligence-driven notification triggers for activity sync, training load, and achievements
#[cfg(feature = "client-notifications")]
pub mod notification_triggers;

/// Background worker that polls scheduled_notifications and fires due notifications via cron
#[cfg(feature = "client-notifications")]
pub mod notification_scheduler;
