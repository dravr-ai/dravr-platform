// ABOUTME: Group coaching business logic for multi-person AI coaching
// ABOUTME: Strategy traits, GroupService, context injection, and digest computation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Groups
//!
//! Business logic for group coaching features. This crate contains:
//!
//! - **Strategy traits** for summarization, aggregation, context building, and tier gating
//! - **`GroupService`** — the central coordinator wiring strategies to repository access
//! - **Context injection** — augments agent system prompts with group-aware information
//! - **Digest computation** — weekly group reports for notifications
//! - **Delegated connections** — [`delegation::DelegationStore`], where every
//!   group lifecycle event ends the links a coach reads members through
//! - **Athlete billing** — [`athlete_billing::AthleteBilling`], the one count
//!   behind both a plan's group allowance and the per-athlete charge above it

// Re-export pierre-core modules for path compatibility within this crate
pub use pierre_core::errors;
pub use pierre_core::models;

/// Strategy traits for group coaching behavior
pub mod strategies;

/// Member-state alert sections (broken / stale) appended to the group context
mod context_alerts;

/// Central group coaching service and context injection
pub mod service;

/// Invite redemption: the checks an invite passes and a coach's attachment
mod invites;

/// Coach setup timing: the first athlete to join a coach's group
mod setup_timing;

/// Ending delegated connections: the one chokepoint every lifecycle event uses
pub mod delegation;

/// Who may create a group: the tenant-role shortcut and the creation policy
pub mod creation_policy;

/// Which agents may answer a group's athletes
pub mod group_agent;

/// The count, cap and per-athlete charge for one owner's athletes
pub mod athlete_billing;

// Re-export key types for consumers
pub use athlete_billing::AthleteBilling;
pub use invites::unambiguous_code;
pub use service::GroupService;
