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
//! - **Context injection** — augments coach system prompts with group-aware information
//! - **Digest computation** — weekly group reports for notifications

// Re-export pierre-core modules for path compatibility within this crate
pub use pierre_core::errors;
pub use pierre_core::models;

/// Strategy traits for group coaching behavior
pub mod strategies;

/// Central group coaching service and context injection
pub mod service;

/// System prompt context builders for LLM injection
pub mod context;

// Re-export key types for consumers
pub use service::GroupService;
