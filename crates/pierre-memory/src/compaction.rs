// ABOUTME: CompactionBlock — a summary of earlier conversation turns
// ABOUTME: Replaces raw history in the prompt once the context window fills up
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A compaction summary of one or more earlier conversation turns.
///
/// When the running prompt approaches the model's context window, the
/// [`ConversationCompactor`] service summarizes the oldest N turns into a
/// single compaction block and stores it. Subsequent prompt assembly injects
/// the block text instead of the raw turns, saving tokens while preserving
/// the conversational thread.
///
/// Blocks are rendered in the chat UI as a collapsible "earlier conversation
/// summary" callout so users can see what was compacted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionBlock {
    /// Stable identifier for this block.
    pub id: String,
    /// Tenant that owns the block.
    pub tenant_id: String,
    /// Conversation the block belongs to.
    pub conversation_id: String,
    /// Summary text that replaces the compacted turns when building prompts.
    pub summary: String,
    /// Token count of the `summary` field, for budget accounting.
    pub summary_tokens: i32,
    /// Approximate token count of the raw turns this block replaced, for
    /// usage accounting and "N tokens saved" UI chips.
    pub original_tokens: i32,
    /// Inclusive ID of the first message this block replaces.
    pub first_message_id: String,
    /// Inclusive ID of the last message this block replaces.
    pub last_message_id: String,
    /// When the block was generated.
    pub created_at: DateTime<Utc>,
}

impl CompactionBlock {
    /// Token savings achieved by this block (`original_tokens - summary_tokens`).
    /// Clamped at zero to avoid reporting negative savings when a long summary
    /// ends up larger than the compacted turns (pathological but possible).
    #[must_use]
    pub const fn tokens_saved(&self) -> i32 {
        let raw = self.original_tokens - self.summary_tokens;
        if raw < 0 {
            0
        } else {
            raw
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CompactionBlock;
    use chrono::Utc;

    fn block(original: i32, summary: i32) -> CompactionBlock {
        CompactionBlock {
            id: "b1".into(),
            tenant_id: "t1".into(),
            conversation_id: "c1".into(),
            summary: "earlier turns summarized".into(),
            summary_tokens: summary,
            original_tokens: original,
            first_message_id: "m1".into(),
            last_message_id: "m5".into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn tokens_saved_positive() {
        assert_eq!(block(500, 120).tokens_saved(), 380);
    }

    #[test]
    fn tokens_saved_clamped_at_zero() {
        assert_eq!(block(100, 200).tokens_saved(), 0);
    }
}
