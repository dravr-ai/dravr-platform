// ABOUTME: feedback.recorded — the one notify event every kind of athlete feedback emits (carnet#1235)
// ABOUTME: Thumbs ratings, channel reactions and feedback-thread messages all reach the team through this one call site
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How the team hears about feedback.
//!
//! An athlete gives feedback three ways: a thumb on an agent reply (with an
//! optional reason after a thumbs-down), an emoji reaction on a channel
//! message, and a message in the in-app feedback thread. Each writes its own
//! row and then calls [`emit_feedback_recorded`], so the Slack rule, the
//! `PostHog` mirror and Cloud Logging see one event shape whatever the surface.
//!
//! The event is tier `product` in the contremaitre catalogue: `PostHog`
//! receives it, the text preview included, only for an athlete who opted in
//! to analytics. Slack and Cloud Logging receive it for everyone, which is
//! why the text is cut to [`PREVIEW_CHARS`] rather than carried whole.

use pierre_core::models::TenantId;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;

/// How many characters of the athlete's text the event carries.
const PREVIEW_CHARS: usize = 280;

/// Where a piece of feedback was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedbackSource {
    /// A thumb on an agent reply in the web or mobile app.
    Thumbs,
    /// An emoji reaction on a messaging channel's copy of a reply.
    Reaction,
    /// A message in the in-app feedback thread.
    Thread,
}

impl FeedbackSource {
    /// The catalogue's spelling of the source.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Thumbs => "thumbs",
            Self::Reaction => "reaction",
            Self::Thread => "thread",
        }
    }
}

/// One piece of feedback, as the team is told about it. Every field is
/// borrowed from the write that just stored it.
#[derive(Debug)]
pub(crate) struct FeedbackRecorded<'write> {
    /// Tenant the athlete gave it in.
    pub(crate) tenant_id: TenantId,
    /// The athlete.
    pub(crate) user_id: &'write str,
    /// How it was given.
    pub(crate) source: FeedbackSource,
    /// The client or channel it came from: `app` for the first-party apps,
    /// the channel's name for a reaction.
    pub(crate) surface: &'write str,
    /// `up` or `down` for a rating; `None` for a thread message.
    pub(crate) rating: Option<&'write str>,
    /// The conversation of the reply it is about, if any.
    pub(crate) conversation_id: Option<&'write str>,
    /// The reply it is about, if any.
    pub(crate) message_id: Option<&'write str>,
    /// The agent that wrote that reply, if known.
    pub(crate) agent_id: Option<&'write str>,
    /// What the athlete wrote, if anything.
    pub(crate) comment: Option<&'write str>,
}

/// Tell the team about one piece of feedback.
pub(crate) fn emit_feedback_recorded(feedback: &FeedbackRecorded<'_>) {
    let comment = feedback
        .comment
        .map(str::trim)
        .filter(|text| !text.is_empty());
    info!(
        target: "notify",
        event = "feedback.recorded",
        user_id = %feedback.user_id,
        tenant_id = %feedback.tenant_id,
        source = feedback.source.as_str(),
        surface = feedback.surface,
        rating = feedback.rating.unwrap_or("none"),
        conversation_id = feedback.conversation_id.unwrap_or("none"),
        message_id = feedback.message_id.unwrap_or("none"),
        agent_id = feedback.agent_id.unwrap_or("none"),
        comment_len = comment.map_or(0, |text| text.chars().count()),
        comment_preview = %comment.map(preview).unwrap_or_default(),
        "athlete feedback recorded"
    );
}

/// The agent behind the conversation a piece of feedback is about, so the
/// team sees which agent the athlete was rating.
///
/// A failed lookup is logged and reads as unknown: the feedback is already
/// stored, and the event is worth sending without the name.
pub(crate) async fn conversation_agent(
    resources: &ServerContext,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> Option<String> {
    match resources
        .common
        .repos
        .chat
        .get_conversation(conversation_id, user_id, tenant_id)
        .await
    {
        Ok(conversation) => conversation.and_then(|c| c.agent_id),
        Err(e) => {
            warn!(error = %e, "feedback: reading the conversation's agent failed");
            None
        }
    }
}

/// The athlete's text on one line, cut to [`PREVIEW_CHARS`] characters.
fn preview(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= PREVIEW_CHARS {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(PREVIEW_CHARS).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_comment_is_kept_whole_on_one_line() {
        assert_eq!(
            preview("  the week view\n\nskips   my ride "),
            "the week view skips my ride",
            "bug: a short comment must keep every word, on one line"
        );
    }

    #[test]
    fn the_last_character_that_fits_is_kept_and_the_next_is_cut() {
        let fits = "é".repeat(PREVIEW_CHARS);
        assert_eq!(
            preview(&fits),
            fits,
            "bug: a comment exactly at the bound must travel whole, with no ellipsis"
        );

        let over = "é".repeat(PREVIEW_CHARS + 1);
        let cut = preview(&over);
        assert_eq!(
            cut,
            format!("{fits}…"),
            "bug: one character past the bound must be cut on a character, not a byte, and marked"
        );
    }

    // Wire values, not a restatement: the dravr-contremaitre catalogue's
    // `feedback.recorded` entry names these three spellings of `source`, and
    // the Slack rule and PostHog insights read them.
    #[test]
    fn sources_spell_the_catalogue_values() {
        for (source, wire) in [
            (FeedbackSource::Thumbs, "thumbs"),
            (FeedbackSource::Reaction, "reaction"),
            (FeedbackSource::Thread, "thread"),
        ] {
            assert_eq!(
                source.as_str(),
                wire,
                "bug: {source:?} must spell the catalogue's value"
            );
        }
    }
}
