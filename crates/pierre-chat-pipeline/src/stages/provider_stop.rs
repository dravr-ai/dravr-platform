// ABOUTME: Tells the athlete when the provider truncated or filtered a reply, and stamps the row for replay
// ABOUTME: Appends a localized caveat after the model's words and cuts it off again when history replays
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Caveat on a reply the provider truncated or filtered.
//!
//! A reply the provider stopped early reaches the pipeline as an ordinary
//! `Ok`: only its `finish_reason` says the answer is a fragment
//! ([`ProviderStop::Truncated`]) or is missing what a content filter removed
//! ([`ProviderStop::Filtered`]). Shown as-is, it reads as a finished answer.
//!
//! The caveat goes after the model's words, behind [`STOP_CAVEAT_SEPARATOR`],
//! and the row is stamped so replay can find it: the model's partial answer
//! re-enters later prompts (a "continue" needs it), the platform's caveat does
//! not — see `MessageRecord::replayable_content`.

use pierre_contremaitre::messaging_strings::{KEY_REPLY_STOP_FILTERED, KEY_REPLY_STOP_TRUNCATED};
use pierre_core::models::{
    FILTERED_REPLY_FINISH_REASON, STOP_CAVEAT_SEPARATOR, TRUNCATED_REPLY_FINISH_REASON,
};
use pierre_llm::provider_stop::ProviderStop;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use tracing::warn;

use super::reply_locale::resolve_banner_locale;
use crate::{ChatPipelineContext, TurnInput};

/// The messaging-strings key of the caveat a stop earns, if any.
#[must_use]
pub const fn caveat_key(stop: ProviderStop) -> Option<&'static str> {
    match stop {
        ProviderStop::Truncated => Some(KEY_REPLY_STOP_TRUNCATED),
        ProviderStop::Filtered => Some(KEY_REPLY_STOP_FILTERED),
        ProviderStop::Complete => None,
    }
}

/// The `finish_reason` stamped on a row carrying a stop caveat, if any.
#[must_use]
pub const fn stamped_finish_reason(stop: ProviderStop) -> Option<&'static str> {
    match stop {
        ProviderStop::Truncated => Some(TRUNCATED_REPLY_FINISH_REASON),
        ProviderStop::Filtered => Some(FILTERED_REPLY_FINISH_REASON),
        ProviderStop::Complete => None,
    }
}

/// The reply with `caveat` appended after the model's words.
#[must_use]
pub fn with_stop_caveat(reply: &str, caveat: &str) -> String {
    format!("{reply}{STOP_CAVEAT_SEPARATOR}{caveat}")
}

/// The stop still describing a reply after a post-processing stage turned
/// `before` into `after`, or [`ProviderStop::Complete`] once the model's
/// words are gone.
///
/// Structural, not lexical: the words survive when the stage kept them whole
/// and added around them (a safety disclaimer before, a verification banner
/// after) or cut them to a prefix (the length cap). Anything else — the
/// blocked-topic reply, the verification block fallback, a style re-ask — is
/// text the provider's stop says nothing about, so no caveat may ride it.
#[must_use]
pub fn kept_through(stop: ProviderStop, before: &str, after: &str) -> ProviderStop {
    if after.contains(before) || (!after.is_empty() && before.starts_with(after)) {
        stop
    } else {
        ProviderStop::Complete
    }
}

/// Tell the athlete when the provider cut the reply short or filtered it, and
/// log what the provider reported ignoring.
///
/// `stop` is post-processing's verdict on the delivered text: it is already
/// [`ProviderStop::Complete`] when that text is platform-written (a withheld
/// reply, a reconnect message, a re-ask's replacement). Returns the text to
/// deliver and persist — the reply with the caveat appended — and the stamp
/// the row carries, the marker replay cuts the caveat off by. An empty reply
/// renders as the localized empty-reply error, which is not the model's
/// fragment either, so it takes no caveat. `result.content` stays the model's
/// words for the follow-through stages that learn from them.
pub(crate) fn caveat_delivered_reply(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    result: &ToolLoopResult,
    stop: ProviderStop,
    locale: &str,
) -> Option<(String, &'static str)> {
    if !result.provider_warnings.is_empty() {
        warn!(
            tenant_id = %input.conversation_tenant_id,
            warnings = ?result.provider_warnings,
            "LLM provider ignored request parameters this turn"
        );
    }
    let key = caveat_key(stop)?;
    let stamp = stamped_finish_reason(stop)?;
    warn!(
        tenant_id = %input.conversation_tenant_id,
        finish_reason = ?result.finish_reason,
        ?stop,
        "LLM provider stopped the reply early"
    );
    if result.content.trim().is_empty() {
        return None;
    }
    let caveat = ctx
        .messaging_strings_registry
        .get(key, &resolve_banner_locale(&result.content, locale));
    Some((with_stop_caveat(&result.content, &caveat), stamp))
}
