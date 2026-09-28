// ABOUTME: The SQLite agent row decoders, emitted from the shared body, and the agent content/request hashes
// ABOUTME: Re-exports the agent type definitions so `database::agents::*` stays one import path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Type definitions for agent database operations
mod types;

pub use types::*;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub use crate::repositories::agent_rows::sqlite::row_to_agent;
pub use crate::repositories::agent_rows::sqlite::row_to_agent_list_item;
pub use crate::repositories::agent_rows::sqlite::row_to_agent_version;

/// Compute hash of content for version tracking
pub(crate) fn compute_content_hash(content: &serde_json::Value) -> String {
    let mut hasher = DefaultHasher::new();
    content.to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Compute a content hash from a `CreateAgentRequest` using `DefaultHasher`.
///
/// Hashes the title, `system_prompt`, tags, and all structured section fields
/// to produce a deterministic 16-character hex string for deduplication.
#[must_use]
pub fn compute_request_hash(request: &CreateAgentRequest) -> String {
    let mut hasher = DefaultHasher::new();
    request.title.hash(&mut hasher);
    request.system_prompt.hash(&mut hasher);
    request.tags.hash(&mut hasher);
    request.purpose.hash(&mut hasher);
    request.instructions.hash(&mut hasher);
    request.example_inputs.hash(&mut hasher);
    request.example_outputs.hash(&mut hasher);
    request.success_criteria.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
