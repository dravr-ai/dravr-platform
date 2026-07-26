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
//! 1. **Under warn threshold (70%) and within the message cap** — do nothing.
//! 2. **Warn band (70%–95%) OR over the message cap** — summarize the oldest N
//!    turns via the LLM, persist a [`CompactionBlock`], and replace those
//!    turns in the outgoing [`ChatMessage`] vector with a single system
//!    message. The block is re-injected on later turns by
//!    `build_llm_messages_with_blocks`, so summarized context survives instead
//!    of being lost. The message cap routes here too (rather than dropping)
//!    because it is an under-estimation backstop, not a reason to lose history.
//! 3. **Above emergency (95% — the hard token cliff)** — sliding-window
//!    fallback: drop the oldest turns outright. This is the one place there is
//!    no budget for a summarizer LLM call before the window would overflow, and
//!    the backstop when summarization can't shrink an over-cap thread.
//!
//! The service intentionally takes a `&mut Vec<ChatMessage>` rather than
//! rebuilding the message list from scratch — the caller owns prompt
//! assembly and the compactor just edits the slice.

use core::iter::once;

use pierre_core::config::CompactionConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::narration::scrub_replayed_narration;
use pierre_core::tokens::estimate_prompt_tokens;
use pierre_database::repositories::{HarnessMemoryRepository, InsertCompactionBlockParams};
use pierre_llm::{ChatMessage, ChatRequest, LlmProvider, MessageRole};
use pierre_memory::CompactionBlock;
use tracing::{info, warn};

use pierre_llm::ChatProvider;

/// Prefix used on compaction summary messages so the UI can render them as
/// "earlier conversation summary" callouts.
pub const COMPACTION_MARKER: &str = "[pierre:compaction]\n";

/// Framing prefix carried by a compaction summary on the wire.
///
/// Summaries ride in a `User` message rather than a `System` one — see the
/// one-system-message invariant in
/// [`build_llm_messages`](../../pierre_chat_pipeline/stages/prompt_builder/index.html) —
/// so the model needs to be told the text is recovered history and not
/// something the athlete just said. Used by both the same-turn splice
/// ([`ConversationCompactor::apply_summary`]) and the later-turn replay in
/// `prompt_builder`, so one summary reads identically on the turn it is
/// created and on every turn after it.
///
/// Distinct from [`COMPACTION_MARKER`], which is a UI render artifact and is
/// deliberately kept off the wire.
pub const REPLAYED_SUMMARY_PREFIX: &str = "[Earlier conversation summary — recovered context, \
     not a new message from the athlete]\n";

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
///
/// `first_id` / `last_id` borrow from the caller's `source_ids` slice — the
/// first and last real history-row id within the compacted range (skipping
/// `None` entries such as the system prompt).
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
        let over_messages = non_system_count(ctx.llm_messages) > self.config.max_messages;

        match decide_action(
            before,
            over_messages,
            self.config.warn_tokens(),
            self.config.emergency_tokens(),
        ) {
            CompactionAction::Slide => Ok(self.run_emergency_sliding(ctx.llm_messages, before)),
            CompactionAction::NoOp => Ok(CompactionOutcome::NoOp {
                estimated_tokens: before,
            }),
            CompactionAction::Summarize => {
                // Summarize the oldest turns into a durable block (re-injected
                // on later turns by `build_llm_messages_with_blocks`) instead of
                // raw-dropping, so the thread keeps its context. `try_summarize`
                // already falls back to sliding on an LLM error. If it returns
                // `NoOp` (too few real turns to compact) while we are still over
                // the message cap, slide as the backstop so the cap is never
                // violated.
                let outcome = self.try_summarize(&mut ctx, before).await?;
                if over_messages && matches!(outcome, CompactionOutcome::NoOp { .. }) {
                    Ok(self.run_emergency_sliding(ctx.llm_messages, before))
                } else {
                    Ok(outcome)
                }
            }
        }
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
        // A summary that scrubbed to empty (pure capability-failure/narration
        // content) must not persist: an empty block still COVERS its range on
        // read, silently deleting that history span. Treat it like a failed
        // summarization instead.
        if summary.is_empty() {
            warn!("Summary empty after narration scrub; falling back to sliding window");
            return Ok(self.run_emergency_sliding(ctx.llm_messages, before));
        }

        self.apply_summary(ctx, plan, &summary, before).await
    }

    /// Pick the compaction range and gather the metadata needed to summarize.
    /// Returns `None` if there isn't enough history to compact.
    fn plan_summary<R: HarnessMemoryRepository + ?Sized>(
        &self,
        ctx: &CompactionContext<'_, R>,
    ) -> Option<SummaryPlan> {
        // `source_ids` is index-aligned with `llm_messages` as built by
        // `build_llm_messages`, and nothing mutates `llm_messages` between that
        // build and here on a compacting turn: the only pre-compaction mutator,
        // `inject_startup_context`, fires solely on the first turn
        // (`history_len == 1`), which never reaches the compaction thresholds.
        // This guard makes that invariant safe — if a future stage ever inserts
        // into `llm_messages` without updating `source_ids`, we skip compaction
        // this turn rather than anchor a block to the wrong rows.
        if ctx.source_ids.len() != ctx.llm_messages.len() {
            return None;
        }
        let range = self.pick_range(ctx.source_ids)?;
        let history_range =
            history_range_for(ctx.source_ids, ctx.llm_messages, range.start, range.end)?;
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

        // `User`, not `System`: the live provider keeps only the first system
        // message and filters the rest, so a mid-list system summary never
        // reached the model at all. Framed with REPLAYED_SUMMARY_PREFIX — not
        // COMPACTION_MARKER, which is a UI-only render artifact — so the model
        // reads it as recovered history rather than as an athlete turn, and so
        // this splice matches the later-turn replay in `prompt_builder` byte
        // for byte.
        let rendered = format!(
            "{REPLAYED_SUMMARY_PREFIX}Earlier conversation summary ({} turns, ~{} → ~{} tokens):\n{summary}",
            plan.range.end - plan.range.start,
            original_tokens,
            summary_tokens,
        );
        replace_range(ctx.llm_messages, &plan.range, ChatMessage::user(&rendered));

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
    /// `source_ids` is index-aligned with `llm_messages`: `None` marks a
    /// non-history entry (the leading system prompt **and** any compaction
    /// summary spliced back in by `build_llm_messages_with_blocks`),
    /// `Some(id)` a real persisted turn. We anchor the compaction window on
    /// real turns only, which both preserves the system prompt and — crucially
    /// — never re-summarizes an already-injected summary (those carry `None`,
    /// so they fall outside the range).
    ///
    /// `start` is the first real turn. We then require the
    /// `summarize_oldest_n` entries starting there to be *contiguously* real
    /// (a defensive guard: an unexpected summary interspersed among the oldest
    /// turns means our window assumptions no longer hold, so we skip
    /// compaction this turn) and at least two more real turns to survive after
    /// the window, so the most recent user+assistant pair stays for the LLM to
    /// reply to.
    fn pick_range(&self, source_ids: &[Option<String>]) -> Option<CompactRange> {
        let n = self.config.summarize_oldest_n;

        // First real turn: skips the system prompt and any leading injected
        // summaries (all `None`).
        let start = source_ids.iter().position(Option::is_some)?;

        // The `n` entries to compact must all be real turns. An interspersed
        // injected summary (`None`) inside the window aborts compaction.
        let window_end = start.checked_add(n)?;
        if window_end > source_ids.len() {
            return None;
        }
        if source_ids[start..window_end].iter().any(Option::is_none) {
            return None;
        }

        // Keep-the-last-pair rule: at least two real turns must remain after
        // the compaction window.
        let real_after = source_ids[window_end..]
            .iter()
            .filter(|id| id.is_some())
            .count();
        if real_after < 2 {
            return None;
        }

        Some(CompactRange {
            start,
            end: window_end,
        })
    }

    /// Sliding-window fallback: delegates to [`sliding_window_to_fit`].
    fn sliding_window(&self, llm_messages: &mut Vec<ChatMessage>) -> usize {
        sliding_window_to_fit(llm_messages, &self.config)
    }
}

/// What [`ConversationCompactor::compact_if_needed`] does this turn, decided
/// purely from the size signals so the routing is unit-testable without an LLM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionAction {
    /// Under every threshold — leave the prompt unchanged.
    NoOp,
    /// Summarize the oldest turns into a durable block (re-injected on later
    /// turns), preserving context.
    Summarize,
    /// Emergency drop of the oldest turns — reserved for the hard token cliff,
    /// where there is no budget for a summarizer LLM call before the prompt
    /// would overflow the window.
    Slide,
}

/// Decide the compaction action from the size signals. Pure; exposed for
/// integration tests.
///
/// The hard token cliff (`before >= emergency_tokens`) slides immediately — a
/// multi-second summarizer call risks overflowing the window first. Otherwise
/// an over-message thread OR a warn-band thread summarizes, persisting a durable
/// block rather than raw-dropping history: the message cap is an
/// under-estimation backstop, not a reason to lose context. Below every
/// threshold is a no-op.
#[must_use]
pub const fn decide_action(
    before: u32,
    over_messages: bool,
    warn_tokens: u32,
    emergency_tokens: u32,
) -> CompactionAction {
    if before >= emergency_tokens {
        CompactionAction::Slide
    } else if over_messages || before >= warn_tokens {
        CompactionAction::Summarize
    } else {
        CompactionAction::NoOp
    }
}

/// Count of non-system messages (the system prompt at index 0, if present, is
/// preserved and excluded from the message cap). Exposed for integration tests.
#[must_use]
pub fn non_system_count(messages: &[ChatMessage]) -> usize {
    let system_count = messages
        .first()
        .map_or(0, |m| usize::from(matches!(m.role, MessageRole::System)));
    messages.len().saturating_sub(system_count)
}

/// Drop the oldest non-system turns until the prompt fits within both the
/// message cap (`config.max_messages`) and the warn-level token budget.
///
/// Walks newest → oldest keeping turns until the next-older one would breach the
/// message cap or the token budget, then drops everything older — always
/// retaining at least the most recent user+assistant pair, and dropping at least
/// `config.sliding_drop_n` so a barely-over thread does not re-trigger emergency
/// on the next turn. Unlike a fixed-count drop, this bounds an already-ballooned
/// thread in a single pass. Returns the number of messages removed. Exposed for
/// integration tests.
pub fn sliding_window_to_fit(
    llm_messages: &mut Vec<ChatMessage>,
    config: &CompactionConfig,
) -> usize {
    let system_count = llm_messages
        .first()
        .map_or(0, |m| usize::from(matches!(m.role, MessageRole::System)));
    let total = llm_messages.len();
    let non_system = total.saturating_sub(system_count);
    let min_keep = non_system.min(2);
    let token_budget = config.warn_tokens();
    let msg_cap = config.max_messages.max(min_keep);

    let mut kept = 0usize;
    let mut tokens = 0u32;
    let mut keep_start = total;
    for i in (system_count..total).rev() {
        let next_kept = kept + 1;
        let next_tokens = tokens.saturating_add(estimate_prompt_tokens(&llm_messages[i].content));
        if next_kept > min_keep && (next_kept > msg_cap || next_tokens > token_budget) {
            break;
        }
        kept = next_kept;
        tokens = next_tokens;
        keep_start = i;
    }

    // Nothing breached the bounds — the thread already fits, so drop nothing
    // (the `sliding_drop_n` floor only applies when we are actually trimming).
    let fit_dropped = keep_start.saturating_sub(system_count);
    if fit_dropped == 0 {
        return 0;
    }

    let max_droppable = non_system.saturating_sub(min_keep);
    let dropped = fit_dropped.max(config.sliding_drop_n).min(max_droppable);
    if dropped == 0 {
        return 0;
    }
    llm_messages.drain(system_count..system_count + dropped);
    dropped
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
    /// Per-message history-row ids, index-aligned with `llm_messages` at the
    /// point `plan_summary` reads them (before any mutation): `None` for the
    /// system prompt, `Some(MessageRecord.id)` for each surviving history row.
    /// Used to anchor the compaction block's `first_message_id` /
    /// `last_message_id` metadata to real persisted rows.
    pub source_ids: &'a [Option<String>],
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

fn history_range_for<'a>(
    source_ids: &'a [Option<String>],
    llm_messages: &[ChatMessage],
    start: usize,
    end: usize,
) -> Option<HistoryRange<'a>> {
    // `source_ids` is index-aligned with `llm_messages`: `None` marks the
    // system prompt, `Some(id)` carries the originating history-row id. We
    // map the compaction range `[start, end)` directly through it — no
    // positional history assumption — because `build_llm_messages` drops
    // history rows that strip to empty, so a position-based remap would be
    // off by the number of dropped rows.
    if end > source_ids.len() || start >= end {
        return None;
    }

    let span = &source_ids[start..end];
    let first_id = span.iter().flatten().next()?;
    let last_id = span.iter().flatten().next_back()?;
    let original_tokens: u32 = llm_messages[start..end]
        .iter()
        .map(|m| estimate_prompt_tokens(&m.content))
        .sum();
    Some(HistoryRange {
        first_id: first_id.as_str(),
        last_id: last_id.as_str(),
        original_tokens,
    })
}

const SUMMARIZER_SYSTEM_PROMPT: &str =
    "You are a conversation summarizer for a fitness coaching assistant. \
     Summarize the following coaching exchange in 2–4 plain-English sentences. \
     Preserve: what the user asked, what the coach said, and any concrete plans, \
     numbers, goals, or commitments mentioned. Omit any meta-commentary about \
     the assistant itself — its tools, data access, capabilities, errors, or \
     technical failures; summarize only the coaching content. Do not add new \
     information. Output only the summary text — no headings, no markdown, no JSON.";

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
    // The summarizer is itself an LLM and can echo capability-failure or
    // narration content from the turns despite the omit instruction; a block
    // is re-injected into every later prompt, so it must be born clean, not
    // only cleaned at injection time.
    Ok(scrub_replayed_narration(response.content.trim()).cleaned)
}

// Unit tests live in crates/pierre-server/tests/conversation_compaction_test.rs
// per the project rule that src/ must not contain inline cfg-test modules.
