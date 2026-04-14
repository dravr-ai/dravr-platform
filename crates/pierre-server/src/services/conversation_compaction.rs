// ABOUTME: Tier 1 conversation compaction — keep long conversations under the context window
// ABOUTME: Pre-flight token count, summarize oldest turns, fall back to sliding window above emergency
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Conversation Compaction
//!
//! Implements the Tier 1 harness compactor described in the coaching harness
//! architecture gist. On every turn the pipeline estimates how big the
//! prompt is about to be, compares it against the model's context window,
//! and takes one of three actions:
//!
//! 1. **Under warn threshold (70%)** — do nothing.
//! 2. **Between warn and emergency (70%–95%)** — summarize the oldest N
//!    turns via the LLM, persist a [`CompactionBlock`], and replace those
//!    turns in the outgoing [`ChatMessage`] vector with a single system
//!    message tagged as a compaction summary.
//! 3. **Above emergency (95%)** — sliding-window fallback: drop the oldest
//!    turns outright. This is a safety net against pathological cases where
//!    summarization doesn't shrink enough.
//!
//! The service intentionally takes a `&mut Vec<ChatMessage>` rather than
//! rebuilding the message list from scratch — the caller owns prompt
//! assembly and the compactor just edits the slice.

use core::iter::once;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::tokens::estimate_prompt_tokens;
use pierre_database::database::MessageRecord;
use pierre_database::repositories::{HarnessMemoryRepository, InsertCompactionBlockParams};
use pierre_llm::{ChatMessage, ChatRequest, LlmProvider, MessageRole};
use pierre_memory::CompactionBlock;
use tracing::{info, warn};

use crate::llm::ChatProvider;

/// Prefix used on compaction summary messages so the UI can render them as
/// "earlier conversation summary" callouts.
pub const COMPACTION_MARKER: &str = "[pierre:compaction]\n";

/// Tuning for the conversation compactor.
///
/// Defaults mirror the coaching harness gist: 128K context window with a
/// 70% warn and 95% emergency threshold. Callers override per-tenant via
/// admin config in Tier 6.
#[derive(Debug, Clone, Copy)]
pub struct CompactionConfig {
    /// Target context window size in tokens. When unset we assume the
    /// conservative floor across Gemini / Groq / local models.
    pub window_tokens: u32,
    /// Fraction of the window that triggers a summarization pass.
    pub warn_threshold: f32,
    /// Fraction of the window that triggers the sliding-window emergency
    /// fallback. Must be strictly greater than `warn_threshold`.
    pub emergency_threshold: f32,
    /// How many of the oldest history turns we summarize when we trigger.
    pub summarize_oldest_n: usize,
    /// How many of the oldest history turns we drop under emergency sliding
    /// window mode.
    pub sliding_drop_n: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            window_tokens: 128_000,
            warn_threshold: 0.70,
            emergency_threshold: 0.95,
            summarize_oldest_n: 6,
            sliding_drop_n: 4,
        }
    }
}

impl CompactionConfig {
    /// Token count at which summarization becomes active.
    ///
    /// Rounded from `window_tokens * warn_threshold` to avoid `f32` precision
    /// artifacts (e.g., `128_000 * 0.70f32` evaluates to 89599.999… in `f64`).
    #[must_use]
    pub fn warn_tokens(&self) -> u32 {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let v = (f64::from(self.window_tokens) * f64::from(self.warn_threshold)).round() as u32;
        v
    }

    /// Token count at which the sliding-window fallback triggers.
    ///
    /// Rounded for the same reason as [`Self::warn_tokens`].
    #[must_use]
    pub fn emergency_tokens(&self) -> u32 {
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let v =
            (f64::from(self.window_tokens) * f64::from(self.emergency_threshold)).round() as u32;
        v
    }
}

/// Outcome of a compaction attempt.
#[derive(Debug, Clone)]
pub enum CompactionOutcome {
    /// Prompt fit under the warn threshold — no change applied.
    NoOp {
        /// Estimated total prompt size in tokens.
        estimated_tokens: u32,
    },
    /// Oldest turns summarized into a persisted compaction block.
    Summarized {
        /// Estimated prompt tokens before the compactor ran.
        estimated_tokens_before: u32,
        /// Estimated prompt tokens after the compactor replaced the range.
        estimated_tokens_after: u32,
        /// Persisted block describing what was compacted.
        block: CompactionBlock,
    },
    /// Emergency sliding window — oldest turns dropped without summarization.
    SlidingWindow {
        /// Estimated prompt tokens before the fallback triggered.
        estimated_tokens_before: u32,
        /// Estimated prompt tokens after the oldest turns were dropped.
        estimated_tokens_after: u32,
        /// Number of messages removed from the head of the non-system slice.
        dropped: usize,
    },
}

impl CompactionOutcome {
    /// Returns the compaction block if the outcome persisted one.
    #[must_use]
    pub const fn block(&self) -> Option<&CompactionBlock> {
        match self {
            Self::Summarized { block, .. } => Some(block),
            Self::NoOp { .. } | Self::SlidingWindow { .. } => None,
        }
    }
}

/// Message range inside the LLM message vector that the compactor will
/// collapse. The vector is always laid out as
/// `[system_prompt?, role:user/assistant/system*...]` so the system prompt
/// at index 0 (if any) is preserved.
struct CompactRange {
    /// Inclusive start index in `llm_messages` of the first turn to compact.
    start: usize,
    /// Exclusive end index.
    end: usize,
}

/// Metadata about which history records the compacted range corresponds to.
struct HistoryRange<'a> {
    first_id: &'a str,
    last_id: &'a str,
    /// Token estimate of the raw turns being replaced.
    original_tokens: u32,
}

/// Everything needed to call the summarizer LLM and persist the result.
///
/// Built once by [`ConversationCompactor::plan_summary`] so the LLM call
/// site doesn't hold any borrow on the message vector.
struct SummaryPlan {
    /// Concatenated `role: content` text of the compacted range, fed to the LLM.
    combined: String,
    /// Index range inside the LLM message vector that will be replaced.
    range: CompactRange,
    /// First persisted message ID covered by the block.
    first_id: String,
    /// Last persisted message ID covered by the block.
    last_id: String,
    /// Approximate token count of the raw turns being replaced.
    original_tokens: u32,
}

/// The conversation compactor.
///
/// Stateless beyond its [`CompactionConfig`]; the LLM and DB handles are
/// passed per call so the same service can be invoked from both the route
/// handler and the messaging pipeline without rebuilding.
pub struct ConversationCompactor {
    config: CompactionConfig,
}

impl ConversationCompactor {
    /// Build a compactor with the given tuning.
    #[must_use]
    pub const fn new(config: CompactionConfig) -> Self {
        Self { config }
    }

    /// Evaluate the prompt and compact if needed.
    ///
    /// # Errors
    ///
    /// Returns the persistence error if inserting the compaction block fails.
    /// LLM summarization failures are converted to sliding-window fallback
    /// with a warning log rather than propagated — losing precise summaries
    /// is better than failing an entire turn.
    pub async fn compact_if_needed<R: HarnessMemoryRepository + ?Sized>(
        &self,
        mut ctx: CompactionContext<'_, R>,
    ) -> AppResult<CompactionOutcome> {
        let before = estimate_messages_tokens(ctx.llm_messages);

        if before < self.config.warn_tokens() {
            return Ok(CompactionOutcome::NoOp {
                estimated_tokens: before,
            });
        }

        if before >= self.config.emergency_tokens() {
            return Ok(self.run_emergency_sliding(ctx.llm_messages, before));
        }

        self.try_summarize(&mut ctx, before).await
    }

    /// Emergency sliding-window fallback. Drops the oldest turns and
    /// returns the resulting outcome.
    fn run_emergency_sliding(
        &self,
        llm_messages: &mut Vec<ChatMessage>,
        before: u32,
    ) -> CompactionOutcome {
        warn!(
            tokens = before,
            window = self.config.window_tokens,
            "Conversation above emergency threshold — sliding window fallback"
        );
        let dropped = self.sliding_window(llm_messages);
        let after = estimate_messages_tokens(llm_messages);
        CompactionOutcome::SlidingWindow {
            estimated_tokens_before: before,
            estimated_tokens_after: after,
            dropped,
        }
    }

    /// Summarize the oldest N turns; on LLM failure, fall back to sliding
    /// window. Returns an error only if persisting the compaction block fails.
    async fn try_summarize<R: HarnessMemoryRepository + ?Sized>(
        &self,
        ctx: &mut CompactionContext<'_, R>,
        before: u32,
    ) -> AppResult<CompactionOutcome> {
        let Some(plan) = self.plan_summary(ctx) else {
            return Ok(CompactionOutcome::NoOp {
                estimated_tokens: before,
            });
        };

        let summary = match summarize_turns(ctx.provider, &plan.combined).await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "Summarization failed; falling back to sliding window");
                return Ok(self.run_emergency_sliding(ctx.llm_messages, before));
            }
        };

        self.apply_summary(ctx, plan, &summary, before).await
    }

    /// Pick the compaction range and gather the metadata needed to summarize.
    /// Returns `None` if there isn't enough history to compact.
    fn plan_summary<R: HarnessMemoryRepository + ?Sized>(
        &self,
        ctx: &CompactionContext<'_, R>,
    ) -> Option<SummaryPlan> {
        let range = self.pick_range(ctx.llm_messages)?;
        let history_range = history_range_for(ctx.history, range.start, range.end)?;
        let combined = extract_range_text(ctx.llm_messages, &range);
        Some(SummaryPlan {
            combined,
            range,
            first_id: history_range.first_id.to_owned(),
            last_id: history_range.last_id.to_owned(),
            original_tokens: history_range.original_tokens,
        })
    }

    /// Persist the compaction block and splice the summary into `llm_messages`.
    async fn apply_summary<R: HarnessMemoryRepository + ?Sized>(
        &self,
        ctx: &mut CompactionContext<'_, R>,
        plan: SummaryPlan,
        summary: &str,
        before: u32,
    ) -> AppResult<CompactionOutcome> {
        let _ = self;
        let summary_tokens = i32::try_from(estimate_prompt_tokens(summary)).unwrap_or(i32::MAX);
        let original_tokens = i32::try_from(plan.original_tokens).unwrap_or(i32::MAX);

        let block = ctx
            .repo
            .insert_compaction_block(&InsertCompactionBlockParams {
                tenant_id: ctx.tenant_id,
                conversation_id: ctx.conversation_id,
                summary,
                summary_tokens,
                original_tokens,
                first_message_id: &plan.first_id,
                last_message_id: &plan.last_id,
            })
            .await?;

        let rendered = format!(
            "{COMPACTION_MARKER}Earlier conversation summary ({} turns, ~{} → ~{} tokens):\n{summary}",
            plan.range.end - plan.range.start,
            original_tokens,
            summary_tokens,
        );
        replace_range(
            ctx.llm_messages,
            &plan.range,
            ChatMessage::system(&rendered),
        );

        let after = estimate_messages_tokens(ctx.llm_messages);
        info!(
            tokens_before = before,
            tokens_after = after,
            summary_tokens,
            original_tokens,
            "Conversation compacted"
        );

        Ok(CompactionOutcome::Summarized {
            estimated_tokens_before: before,
            estimated_tokens_after: after,
            block,
        })
    }

    /// Choose which range of `llm_messages` indices to compact.
    ///
    /// We always preserve the system prompt at index 0 (if any) and we avoid
    /// dropping the most recent user message. That leaves the oldest user +
    /// assistant turns as compaction candidates.
    fn pick_range(&self, llm_messages: &[ChatMessage]) -> Option<CompactRange> {
        let system_count = llm_messages
            .iter()
            .take(1)
            .filter(|m| matches!(m.role, MessageRole::System))
            .count();
        let non_system_len = llm_messages.len().saturating_sub(system_count);

        // Need at least `summarize_oldest_n + 2` non-system messages so we
        // leave the last user+assistant pair alone for the LLM to reply to.
        if non_system_len < self.config.summarize_oldest_n + 2 {
            return None;
        }

        let start = system_count;
        let end = start + self.config.summarize_oldest_n;
        Some(CompactRange { start, end })
    }

    /// Sliding-window fallback: drop the oldest turns outright.
    fn sliding_window(&self, llm_messages: &mut Vec<ChatMessage>) -> usize {
        let system_count = llm_messages
            .first()
            .map_or(0, |m| usize::from(matches!(m.role, MessageRole::System)));
        let drop_end = (system_count + self.config.sliding_drop_n).min(llm_messages.len());
        if drop_end <= system_count {
            return 0;
        }
        llm_messages.drain(system_count..drop_end);
        drop_end - system_count
    }
}

/// Everything the compactor needs per call.
///
/// Packaged into a struct so the public entry point doesn't grow a six-arg
/// method signature.
pub struct CompactionContext<'a, R: HarnessMemoryRepository + ?Sized> {
    /// Repository for persisting compaction blocks.
    pub repo: &'a R,
    /// LLM provider used to generate the summary text.
    pub provider: &'a ChatProvider,
    /// Tenant that owns the conversation.
    pub tenant_id: TenantId,
    /// Conversation being compacted.
    pub conversation_id: &'a str,
    /// History records underlying the prompt — used to anchor the compaction
    /// block's `first_message_id` / `last_message_id` metadata.
    pub history: &'a [MessageRecord],
    /// The LLM message list being assembled; the compactor mutates it in place.
    pub llm_messages: &'a mut Vec<ChatMessage>,
}

/// Sum-of-content token estimate across all chat messages.
///
/// Exposed for integration tests; callers should not rely on this as a
/// substitute for real provider usage counts.
#[must_use]
pub fn estimate_messages_tokens(messages: &[ChatMessage]) -> u32 {
    messages
        .iter()
        .map(|m| estimate_prompt_tokens(&m.content))
        .sum()
}

fn extract_range_text(messages: &[ChatMessage], range: &CompactRange) -> String {
    let mut out = String::new();
    for msg in &messages[range.start..range.end] {
        let role = match msg.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
            MessageRole::Tool => "tool",
        };
        out.push_str(role);
        out.push_str(": ");
        out.push_str(&msg.content);
        out.push('\n');
    }
    out
}

fn replace_range(messages: &mut Vec<ChatMessage>, range: &CompactRange, replacement: ChatMessage) {
    messages.splice(range.start..range.end, once(replacement));
}

fn history_range_for(
    history: &[MessageRecord],
    start: usize,
    end: usize,
) -> Option<HistoryRange<'_>> {
    // The llm_messages vector is laid out as [system?, ...history],
    // so subtract the system offset to land back in history coordinates.
    // We don't strictly know if there's a system message — assume yes when
    // start > 0; callers always build messages with a system prompt in
    // practice.
    let offset = start.min(1);
    let h_start = start.saturating_sub(offset);
    let h_end = end.saturating_sub(offset);

    if h_start >= history.len() || h_end > history.len() || h_start >= h_end {
        return None;
    }

    let slice = &history[h_start..h_end];
    let first = slice.first()?;
    let last = slice.last()?;
    let original_tokens: u32 = slice
        .iter()
        .map(|m| estimate_prompt_tokens(&m.content))
        .sum();
    Some(HistoryRange {
        first_id: &first.id,
        last_id: &last.id,
        original_tokens,
    })
}

const SUMMARIZER_SYSTEM_PROMPT: &str =
    "You are a conversation summarizer for a fitness coaching assistant. \
     Summarize the following coaching exchange in 2–4 plain-English sentences. \
     Preserve: what the user asked, what the coach said, and any concrete plans, \
     numbers, goals, or commitments mentioned. Do not add new information. \
     Output only the summary text — no headings, no markdown, no JSON.";

async fn summarize_turns(provider: &ChatProvider, turns_text: &str) -> AppResult<String> {
    let messages = vec![
        ChatMessage::system(SUMMARIZER_SYSTEM_PROMPT),
        ChatMessage::user(turns_text),
    ];
    // Low temperature for consistent condensation
    let request = ChatRequest::new(messages).with_temperature(0.2);

    let response = LlmProvider::complete(provider, &request)
        .await
        .map_err(|e| {
            AppError::external_service("compactor", format!("summarization failed: {e}"))
        })?;
    Ok(response.content.trim().to_owned())
}

// Unit tests live in crates/pierre-server/tests/conversation_compaction_test.rs
// per the project rule that src/ must not contain inline cfg-test modules.
