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
//!
//! `feedback_id` names the row the feedback was stored as, so the Reply and
//! Show full text buttons on its Slack card can find it again (carnet#1392):
//! the thread message for `thread`, the `chat_message_feedback` row for
//! `thumbs` and `reaction`.

use bytes::Bytes;
use dravr_tronc::notifications::{SlackClient, SlackConfig, SlackFile, SlackResult};
use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
use pierre_core::models::TenantId;
use secrecy::ExposeSecret;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;

/// The event every kind of feedback emits, whose routing rule names the
/// feedback room: where a screenshot is shared, and where the card's Reply
/// and Show full text buttons are honoured.
pub(crate) const FEEDBACK_RECORDED_EVENT: &str = "feedback.recorded";

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
    /// The row it was stored as: the thread message for a thread message,
    /// the `chat_message_feedback` row for a rating or a reaction.
    pub(crate) feedback_id: &'write str,
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
    /// Whether a screenshot came with it, which is shared into the room.
    pub(crate) has_image: bool,
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
        feedback_id = %feedback.feedback_id,
        source = feedback.source.as_str(),
        surface = feedback.surface,
        rating = feedback.rating.unwrap_or("none"),
        conversation_id = feedback.conversation_id.unwrap_or("none"),
        message_id = feedback.message_id.unwrap_or("none"),
        agent_id = feedback.agent_id.unwrap_or("none"),
        comment_len = comment.map_or(0, |text| text.chars().count()),
        comment_preview = %comment.map(preview).unwrap_or_default(),
        has_image = feedback.has_image,
        "athlete feedback recorded"
    );
}

/// Share the screenshot of feedback `feedback_id` into the feedback room,
/// beside its card: uploaded into the workspace with Slack's file upload, so
/// the athlete's image never sits behind a public URL.
///
/// Fire-and-forget, after the feedback is stored: the athlete's message
/// stands whether or not Slack takes the file. The upload runs on the
/// server's in-flight task tracker, so a shutdown drain waits for it rather
/// than cutting it off mid-upload. The room is the one the
/// `feedback.recorded` rule names, never the routing's default room, and a
/// deployment without a bot token or that rule shares nothing.
pub(crate) fn share_feedback_image(
    resources: &ServerContext,
    feedback_id: &str,
    media_type: &str,
    bytes: Bytes,
) {
    let Some(bot_token) = resources.common.config.webhooks.slack_bot_token.as_ref() else {
        warn!(
            feedback_id,
            "SLACK_BOT_TOKEN not configured; the feedback screenshot was not shared"
        );
        return;
    };
    let Some(room) = NOTIFY_ROUTING_PROVIDER.event_channel(FEEDBACK_RECORDED_EVENT) else {
        warn!(
            feedback_id,
            "no routing rule names the feedback room; the screenshot was not shared"
        );
        return;
    };
    let client = SlackClient::new(&SlackConfig {
        bot_token: bot_token.expose_secret().to_owned(),
        error_channel: String::new(),
        signing_secret: None,
    });
    let extension = media_type.strip_prefix("image/").unwrap_or("png");
    let file = SlackFile::new(
        format!("feedback-{feedback_id}.{extension}"),
        Vec::from(bytes),
    )
    .with_title("Screenshot")
    .with_alt_text("A screenshot an athlete attached to their feedback");
    let comment = format!("Screenshot attached to feedback `{feedback_id}`");
    let feedback_id = feedback_id.to_owned();
    resources.common.turns.spawn(async move {
        match client.upload_file(&room, file, Some(&comment)).await {
            SlackResult::Ok => info!(
                feedback_id,
                "feedback screenshot shared into the feedback room"
            ),
            failed => warn!(feedback_id, outcome = ?failed, "feedback screenshot not shared"),
        }
    });
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

/// A text on one line, cut to [`PREVIEW_CHARS`] characters: the athlete's on
/// the event, and the team's reply on the card it answers.
pub(crate) fn preview(text: &str) -> String {
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
