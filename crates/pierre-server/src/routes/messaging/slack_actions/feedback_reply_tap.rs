// ABOUTME: Answer athlete feedback from its Slack card: Reply opens a modal whose submission lands in the athlete's thread
// ABOUTME: Show full text opens a read-only modal, so the athlete's whole text never sits in the room's history (carnet#1392)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Replying to feedback from Slack.
//!
//! A `feedback.recorded` card carries two buttons ([`super::OpsDecisionActions`]):
//!
//! - **Reply** opens a modal showing what the athlete wrote above a reply
//!   field. Its submission is stored as a team row of the athlete's feedback
//!   thread ([`FeedbackWriter::Team`]), which the web app shows signed "Dravr
//!   team" and counts as unread until the athlete opens the thread. The card
//!   then says who replied and what, keeping its buttons for a follow-up.
//! - **Show full text** opens the same text in a read-only modal: the card
//!   carries a 280-character preview, and the whole text is seen only by
//!   whoever asks for it.
//!
//! A reply to a thumbs rating or a reaction, which has no thread row, starts
//! the thread with the rated agent reply as its `about`.
//!
//! The room is the trust boundary, as for every ops decision: a tap is
//! honoured only in the room the `feedback.recorded` rule names, and the
//! submission re-checks that room from the modal's `private_metadata`, which
//! only this server writes and Slack returns inside a signed request.

use axum::http::StatusCode;
use axum::Json;
use chrono::Utc;
use dravr_tronc::notifications::SlackResult;
use pierre_contremaitre::notify_routing::NOTIFY_ROUTING_PROVIDER;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    FeedbackAbout, FeedbackMessage, FeedbackWriter, NewFeedbackMessage,
};
use pierre_database::RepositoryRegistry;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{info, warn};
use uuid::Uuid;

use super::ops_card::{is_refusal, keep_card_with_reason, room_text};
use super::ops_decisions::verify_ops_room;
use super::{build_slack_client, configured_bot_token, update_card_at, SlackAction};
use crate::mcp::resources::ServerContext;
// The longest reply the modal accepts, in characters, is the thread's own bound.
use crate::routes::feedback::MAX_BODY_CHARS as MAX_REPLY_CHARS;
use crate::services::feedback_events::{self, FEEDBACK_RECORDED_EVENT};

/// The most text one `plain_text` section block holds.
const SECTION_TEXT_CHARS: usize = 3000;

/// The most characters Slack keeps in a view's `private_metadata`.
const PRIVATE_METADATA_CHARS: usize = 3000;

/// `callback_id` of the reply modal, which names its submission.
pub(super) const REPLY_CALLBACK_ID: &str = "feedback_reply";

/// `block_id` and `action_id` of the reply field.
const REPLY_BLOCK_ID: &str = "feedback_reply";
const REPLY_ACTION_ID: &str = "body";

/// `block_id` of the line a reply adds to the card; the next reply replaces it.
const REPLIED_BLOCK_ID: &str = "feedback-replied";

/// Which row a feedback card names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FeedbackKind {
    /// A message in the athlete's feedback thread.
    Thread,
    /// A thumbs rating or a channel reaction on an agent reply.
    Rating,
}

impl FeedbackKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Rating => "rating",
        }
    }

    /// The kind of row a `feedback.recorded` `source` was stored as.
    pub(super) fn of_source(source: &str) -> Option<Self> {
        match source {
            "thread" => Some(Self::Thread),
            "thumbs" | "reaction" => Some(Self::Rating),
            _ => None,
        }
    }
}

/// The piece of feedback a button or a modal acts on: its kind, and the
/// athlete, tenant and row it was stored under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FeedbackRef {
    pub(super) kind: FeedbackKind,
    pub(super) tenant_id: Uuid,
    pub(super) user_id: Uuid,
    pub(super) id: Uuid,
}

impl FeedbackRef {
    /// `kind:tenant:user:id`, the shape [`Self::parse`] reads back.
    pub(super) fn encode(self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.kind.as_str(),
            self.tenant_id,
            self.user_id,
            self.id
        )
    }

    /// Read back what [`Self::encode`] wrote.
    ///
    /// # Errors
    /// Returns an invalid-input error for anything else.
    pub(super) fn parse(raw: &str) -> AppResult<Self> {
        let invalid = || AppError::invalid_input(format!("Invalid feedback reference: {raw}"));
        let mut parts = raw.split(':');
        let kind = match parts.next() {
            Some("thread") => FeedbackKind::Thread,
            Some("rating") => FeedbackKind::Rating,
            _ => return Err(invalid()),
        };
        let mut uuid = || {
            parts
                .next()
                .and_then(|part| Uuid::parse_str(part).ok())
                .ok_or_else(invalid)
        };
        let feedback = Self {
            kind,
            tenant_id: uuid()?,
            user_id: uuid()?,
            id: uuid()?,
        };
        if parts.next().is_some() {
            return Err(invalid());
        }
        Ok(feedback)
    }
}

/// Which of the card's two buttons was tapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FeedbackTap {
    /// Open the reply modal.
    Reply,
    /// Open the read-only modal with the whole text.
    FullText,
}

/// What the athlete gave, as the modals show it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GivenFeedback {
    /// Their words: the thread message, or a rating's reason.
    text: Option<String>,
    /// `up` or `down` for a rating.
    rating: Option<String>,
    /// The agent reply a rating is about, which a reply to it is about too.
    rated_reply: Option<FeedbackAbout>,
    /// Whether a screenshot came with it, shared into the room beside the card.
    has_image: bool,
}

/// Read the feedback `feedback` names, in its own tenant and for its own
/// athlete.
///
/// # Errors
/// Returns not-found when the row is gone (the athlete deleted their account,
/// or took a rating back) and a database error when it cannot be read.
async fn given_feedback(
    repos: &RepositoryRegistry,
    feedback: FeedbackRef,
) -> AppResult<GivenFeedback> {
    let tenant_id = TenantId::from_uuid(feedback.tenant_id);
    let id = feedback.id.to_string();
    let gone = || AppError::not_found("This feedback no longer exists");
    match feedback.kind {
        FeedbackKind::Thread => {
            let message = repos
                .feedback_messages
                .get_feedback_message(tenant_id, feedback.user_id, &id)
                .await?
                .ok_or_else(gone)?;
            Ok(GivenFeedback {
                has_image: message.image.is_some(),
                text: Some(message.body),
                rating: None,
                rated_reply: None,
            })
        }
        FeedbackKind::Rating => {
            let record = repos
                .chat
                .get_message_feedback(&id, &feedback.user_id.to_string(), tenant_id)
                .await?
                .ok_or_else(gone)?;
            Ok(GivenFeedback {
                text: record.comment,
                rating: Some(record.rating),
                rated_reply: Some(FeedbackAbout {
                    conversation_id: record.conversation_id,
                    message_id: record.message_id,
                }),
                has_image: false,
            })
        }
    }
}

/// Store `body` as the team's reply in the athlete's thread, sent by
/// `replier_slack_user_id`.
///
/// # Errors
/// Refuses an empty or oversized reply, and a reply to feedback that no
/// longer exists; propagates a failed write.
async fn store_team_reply(
    repos: &RepositoryRegistry,
    feedback: FeedbackRef,
    replier_slack_user_id: &str,
    body: &str,
) -> AppResult<FeedbackMessage> {
    let body = body.trim();
    if body.is_empty() {
        return Err(AppError::invalid_input("A reply cannot be empty"));
    }
    if body.chars().count() > MAX_REPLY_CHARS {
        return Err(AppError::invalid_input(format!(
            "A reply holds at most {MAX_REPLY_CHARS} characters"
        )));
    }
    let given = given_feedback(repos, feedback).await?;
    repos
        .feedback_messages
        .insert_feedback_message(&NewFeedbackMessage {
            tenant_id: TenantId::from_uuid(feedback.tenant_id),
            user_id: feedback.user_id,
            writer: FeedbackWriter::Team {
                slack_user_id: replier_slack_user_id,
            },
            body,
            screen: None,
            app_version: None,
            locale: None,
            about: given.rated_reply.as_ref(),
            image: None,
            at: Utc::now(),
        })
        .await
}

/// Where the reply modal came from, carried through Slack in the modal's
/// `private_metadata` so the submission can re-check the room and redraw the
/// card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReplyOrigin {
    /// [`FeedbackRef::encode`] of the feedback replied to.
    feedback: String,
    channel_id: String,
    channel_name: String,
    message_ts: String,
    /// The card as the tap carried it back; empty when it would not fit.
    card: Vec<Value>,
}

impl ReplyOrigin {
    /// The origin as `private_metadata`, dropping the card when the whole
    /// would pass Slack's bound: the reply still lands, the card just stays
    /// as it was.
    fn to_metadata(&self) -> String {
        let whole = serde_json::to_string(self).unwrap_or_default();
        if whole.chars().count() <= PRIVATE_METADATA_CHARS {
            return whole;
        }
        let bare = Self {
            card: Vec::new(),
            ..self.clone()
        };
        serde_json::to_string(&bare).unwrap_or_default()
    }
}

/// The athlete's words as `plain_text` sections, each within Slack's bound:
/// plain text, so nothing they wrote is read as a mention or a link.
fn text_sections(text: &str) -> Vec<Value> {
    let chars: Vec<char> = text.chars().collect();
    chars
        .chunks(SECTION_TEXT_CHARS)
        .map(|chunk| {
            json!({
                "type": "section",
                "text": { "type": "plain_text", "text": chunk.iter().collect::<String>() }
            })
        })
        .collect()
}

/// What the modals show of the feedback: where it came from, then the
/// athlete's words.
fn feedback_blocks(given: &GivenFeedback) -> Vec<Value> {
    let origin = match given.rating.as_deref() {
        Some("up") => "A thumbs up on an agent reply",
        Some(_) => "A thumbs down on an agent reply",
        None => "A message in the athlete's feedback thread",
    };
    let mut blocks = vec![json!({
        "type": "context",
        "elements": [{ "type": "mrkdwn", "text": origin }]
    })];
    match given
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        Some(text) => blocks.extend(text_sections(text)),
        None => blocks.push(json!({
            "type": "context",
            "elements": [{ "type": "mrkdwn", "text": "_No words came with it._" }]
        })),
    }
    if given.has_image {
        blocks.push(json!({
            "type": "context",
            "elements": [{
                "type": "mrkdwn",
                "text": ":frame_with_picture: A screenshot came with it, shared in this room beside the card."
            }]
        }));
    }
    blocks
}

/// The reply modal.
fn reply_view(given: &GivenFeedback, origin: &ReplyOrigin) -> Value {
    let mut blocks = feedback_blocks(given);
    blocks.push(json!({ "type": "divider" }));
    blocks.push(json!({
        "type": "input",
        "block_id": REPLY_BLOCK_ID,
        "label": { "type": "plain_text", "text": "Your reply" },
        "hint": {
            "type": "plain_text",
            "text": "The athlete reads it in their feedback thread, signed Dravr team."
        },
        "element": {
            "type": "plain_text_input",
            "action_id": REPLY_ACTION_ID,
            "multiline": true,
            "max_length": MAX_REPLY_CHARS,
        }
    }));
    json!({
        "type": "modal",
        "callback_id": REPLY_CALLBACK_ID,
        "private_metadata": origin.to_metadata(),
        "title": { "type": "plain_text", "text": "Reply to feedback" },
        "submit": { "type": "plain_text", "text": "Send reply" },
        "close": { "type": "plain_text", "text": "Cancel" },
        "blocks": blocks,
    })
}

/// The read-only modal with the whole text.
fn full_text_view(given: &GivenFeedback) -> Value {
    json!({
        "type": "modal",
        "title": { "type": "plain_text", "text": "Athlete feedback" },
        "close": { "type": "plain_text", "text": "Close" },
        "blocks": feedback_blocks(given),
    })
}

/// Escape what Slack's mrkdwn would read as markup in text the team wrote.
fn escape_mrkdwn(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The card after a reply: as posted, buttons kept for a follow-up, with
/// who replied and what in place of any earlier reply line, and without the
/// line an earlier refused tap left, which the reply has settled. `None` when
/// the modal carried no card to redraw.
fn replied_card(posted: &[Value], replier: &str, reply: &str) -> Option<Value> {
    if posted.is_empty() {
        return None;
    }
    let mut blocks: Vec<Value> = posted
        .iter()
        .filter(|block| {
            block.get("block_id").and_then(Value::as_str) != Some(REPLIED_BLOCK_ID)
                && !is_refusal(block)
        })
        .cloned()
        .collect();
    blocks.push(json!({
        "type": "context",
        "block_id": REPLIED_BLOCK_ID,
        "elements": [{
            "type": "mrkdwn",
            "text": format!(
                ":leftwards_arrow_with_hook: {replier} replied: {}",
                escape_mrkdwn(&feedback_events::preview(reply))
            )
        }]
    }));
    Some(Value::Array(blocks))
}

/// A reply modal's submission, read from a `view_submission` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplySubmission {
    slack_user_id: String,
    slack_username: String,
    origin: ReplyOrigin,
    reply: String,
}

/// Read a reply modal's submission.
///
/// # Errors
/// Returns an invalid-input error for a submission of another view, or one
/// missing its submitter, its origin or its reply field.
fn parse_reply_submission(payload: &Value) -> AppResult<ReplySubmission> {
    let callback_id = payload
        .pointer("/view/callback_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if callback_id != REPLY_CALLBACK_ID {
        return Err(AppError::invalid_input(format!(
            "Unknown view submission: {callback_id}"
        )));
    }
    let slack_user_id = payload
        .pointer("/user/id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("Missing user ID in view submission"))?;
    let slack_username = payload
        .pointer("/user/username")
        .and_then(Value::as_str)
        .or_else(|| payload.pointer("/user/name").and_then(Value::as_str))
        .unwrap_or(slack_user_id);
    let metadata = payload
        .pointer("/view/private_metadata")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let origin: ReplyOrigin = serde_json::from_str(metadata)
        .map_err(|e| AppError::invalid_input(format!("Invalid reply origin: {e}")))?;
    let reply = payload
        .pointer(&format!(
            "/view/state/values/{REPLY_BLOCK_ID}/{REPLY_ACTION_ID}/value"
        ))
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(ReplySubmission {
        slack_user_id: slack_user_id.to_owned(),
        slack_username: slack_username.to_owned(),
        origin,
        reply: reply.to_owned(),
    })
}

/// Open `view` for the tap that carried `trigger_id`.
///
/// # Errors
/// Fails without a bot token, and with Slack's own reason when the modal does
/// not open.
async fn open_view(resources: &ServerContext, trigger_id: &str, view: &Value) -> AppResult<()> {
    let bot_token = configured_bot_token(resources)
        .ok_or_else(|| AppError::internal("SLACK_BOT_TOKEN not configured"))?;
    if trigger_id.is_empty() {
        return Err(AppError::invalid_input("The tap carried no trigger_id"));
    }
    match build_slack_client(bot_token.expose_secret(), resources)
        .open_view(trigger_id, view)
        .await
    {
        SlackResult::Ok => Ok(()),
        SlackResult::ApiError(reason) | SlackResult::HttpError(reason) => Err(
            AppError::invalid_input(format!("Slack did not open the window ({reason})")),
        ),
        _ => Err(AppError::invalid_input("Slack did not open the window")),
    }
}

/// Open the modal `tap` asks for on the feedback `feedback` names.
///
/// # Errors
/// Fails when the feedback is gone or cannot be read, and when the modal does
/// not open ([`open_view`]).
async fn open_feedback_modal(
    resources: &ServerContext,
    action: &SlackAction,
    tap: FeedbackTap,
    feedback: FeedbackRef,
) -> AppResult<()> {
    let given = given_feedback(&resources.common.repos, feedback).await?;
    let view = match tap {
        FeedbackTap::Reply => reply_view(
            &given,
            &ReplyOrigin {
                feedback: feedback.encode(),
                channel_id: action.channel_id.clone(),
                channel_name: action.channel_name.clone(),
                message_ts: action.message_ts.clone(),
                card: action.message_blocks.clone(),
            },
        ),
        FeedbackTap::FullText => full_text_view(&given),
    };
    open_view(resources, &action.trigger_id, &view).await
}

/// Answer a Reply or Show full text tap on a feedback card by opening its
/// modal. A modal that cannot open — the feedback is gone, the trigger
/// expired — says why under the card's buttons.
pub(super) async fn answer_feedback_tap(
    resources: &ServerContext,
    action: &SlackAction,
    tap: FeedbackTap,
    feedback: FeedbackRef,
) -> AppResult<(StatusCode, Json<Value>)> {
    match open_feedback_modal(resources, action, tap, feedback).await {
        Ok(()) => Ok((StatusCode::OK, Json(json!({ "status": "ok" })))),
        Err(e) => {
            warn!(error = %e, feedback_id = %feedback.id, "feedback modal not opened");
            keep_card_with_reason(
                resources,
                action,
                &format!("@{}: {}", action.slack_username, room_text(&e)),
            );
            Err(e)
        }
    }
}

/// The answer that keeps the modal open with `reason` under the reply field.
fn refused_submission(reason: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(json!({
            "response_action": "errors",
            "errors": { REPLY_BLOCK_ID: reason }
        })),
    )
}

/// Answer a reply modal's submission: store the reply in the athlete's
/// thread, redraw the card with who replied and what, and close the modal.
/// A refused or failed reply keeps the modal open and says why, so the typed
/// text is not lost.
///
/// # Errors
/// Returns an invalid-input error for a submission this server never opened.
pub(super) async fn answer_reply_submission(
    resources: &ServerContext,
    payload: &Value,
) -> AppResult<(StatusCode, Json<Value>)> {
    let submission = parse_reply_submission(payload)?;
    let origin = &submission.origin;
    if let Err(e) = verify_ops_room(
        &origin.channel_name,
        &origin.channel_id,
        FEEDBACK_RECORDED_EVENT,
        NOTIFY_ROUTING_PROVIDER.as_ref(),
    ) {
        return Ok(refused_submission(room_text(&e)));
    }
    let feedback = FeedbackRef::parse(&origin.feedback)?;

    match store_team_reply(
        &resources.common.repos,
        feedback,
        &submission.slack_user_id,
        &submission.reply,
    )
    .await
    {
        Ok(stored) => {
            info!(
                feedback_id = %feedback.id,
                kind = feedback.kind.as_str(),
                reply_id = %stored.id,
                slack_user = %submission.slack_user_id,
                reply_len = stored.body.chars().count(),
                "feedback reply sent from Slack"
            );
            let replier = format!("@{}", submission.slack_username);
            if let Some(card) = replied_card(&origin.card, &replier, &stored.body) {
                update_card_at(resources, &origin.channel_id, &origin.message_ts, &card);
            } else {
                warn!(
                    feedback_id = %feedback.id,
                    "the reply modal carried no card; the Slack card was not redrawn"
                );
            }
            Ok((StatusCode::OK, Json(json!({ "response_action": "clear" }))))
        }
        Err(e) => {
            warn!(error = %e, feedback_id = %feedback.id, "feedback reply not stored");
            Ok(refused_submission(room_text(&e)))
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::errors::ErrorCode;
    use pierre_core::models::{AddMessageParams, TenantId, UpsertMessageFeedbackParams};
    use pierre_core::transport::TransportPolicy;
    use pierre_database::repositories::{FeedbackAuthor, FeedbackWriter, NewFeedbackMessage};
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;
    use serde_json::{json, Value};
    use uuid::Uuid;

    use super::{
        full_text_view, given_feedback, parse_reply_submission, replied_card, reply_view,
        store_team_reply, text_sections, FeedbackKind, FeedbackRef, GivenFeedback, ReplyOrigin,
        PRIVATE_METADATA_CHARS, REPLIED_BLOCK_ID, REPLY_CALLBACK_ID,
    };

    fn a_ref(kind: FeedbackKind) -> FeedbackRef {
        FeedbackRef {
            kind,
            tenant_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            id: Uuid::new_v4(),
        }
    }

    #[test]
    fn a_feedback_reference_reads_back_as_it_was_written() {
        for kind in [FeedbackKind::Thread, FeedbackKind::Rating] {
            let feedback = a_ref(kind);
            assert_eq!(
                FeedbackRef::parse(&feedback.encode()).ok(),
                Some(feedback),
                "{kind:?}"
            );
        }
        let id = Uuid::new_v4();
        for broken in [
            format!("thread:{id}:{id}"),
            format!("thread:{id}:{id}:{id}:{id}"),
            format!("note:{id}:{id}:{id}"),
            format!("rating:{id}:not-a-uuid:{id}"),
            String::new(),
        ] {
            assert!(FeedbackRef::parse(&broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn a_source_names_the_row_it_was_stored_as() {
        assert_eq!(
            FeedbackKind::of_source("thread"),
            Some(FeedbackKind::Thread),
            "bug: a thread message is a feedback_messages row"
        );
        assert_eq!(
            FeedbackKind::of_source("thumbs"),
            Some(FeedbackKind::Rating),
            "bug: a thumbs rating is a chat_message_feedback row"
        );
        assert_eq!(
            FeedbackKind::of_source("reaction"),
            Some(FeedbackKind::Rating),
            "bug: a reaction is stored as a rating"
        );
        assert_eq!(
            FeedbackKind::of_source("survey"),
            None,
            "bug: a source no row is stored under names no kind"
        );
    }

    #[test]
    fn the_whole_text_is_shown_as_plain_text_within_slacks_bound() {
        let text = format!("{}<!channel>", "é".repeat(3000));
        let sections = text_sections(&text);
        assert_eq!(sections.len(), 2, "3010 characters take two sections");
        for section in &sections {
            assert_eq!(
                section["text"]["type"], "plain_text",
                "never read as mrkdwn"
            );
        }
        let joined: String = sections
            .iter()
            .map(|s| s["text"]["text"].as_str().unwrap())
            .collect();
        assert_eq!(joined, text, "nothing is cut or lost at the seam");
    }

    fn given(text: Option<&str>, rating: Option<&str>) -> GivenFeedback {
        GivenFeedback {
            text: text.map(str::to_owned),
            rating: rating.map(str::to_owned),
            rated_reply: None,
            has_image: false,
        }
    }

    fn origin(card: Vec<Value>) -> ReplyOrigin {
        ReplyOrigin {
            feedback: a_ref(FeedbackKind::Thread).encode(),
            channel_id: "C0FEEDBACK".to_owned(),
            channel_name: "dravr-feedback".to_owned(),
            message_ts: "1730000000.000100".to_owned(),
            card,
        }
    }

    fn a_card() -> Vec<Value> {
        vec![
            json!({ "type": "section", "block_id": "notify-feedback.recorded",
                    "text": { "type": "mrkdwn", "text": "feedback recorded" } }),
            json!({ "type": "actions", "block_id": "notify-actions-feedback.recorded",
                    "elements": [{ "type": "button", "action_id": "feedback_reply:x" }] }),
        ]
    }

    #[test]
    fn the_reply_modal_shows_the_feedback_and_carries_where_it_came_from() {
        let from = origin(a_card());
        let view = reply_view(&given(Some("The map is slow."), None), &from);
        assert_eq!(
            view["callback_id"], REPLY_CALLBACK_ID,
            "bug: the submission is recognised by the modal's callback_id"
        );
        let texts: Vec<&str> = view["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["text"]["text"].as_str())
            .collect();
        assert_eq!(
            texts,
            ["The map is slow."],
            "the athlete's words sit above the field"
        );
        let carried: ReplyOrigin =
            serde_json::from_str(view["private_metadata"].as_str().unwrap()).unwrap();
        assert_eq!(
            carried, from,
            "the submission gets back the room and the card"
        );

        let read_only = full_text_view(&given(None, Some("down")));
        assert!(
            read_only.get("submit").is_none(),
            "Show full text has nothing to send"
        );
        assert_eq!(
            read_only["blocks"][0]["elements"][0]["text"], "A thumbs down on an agent reply",
            "bug: the modal says what kind of feedback it shows"
        );
    }

    #[test]
    fn a_card_too_large_to_carry_is_dropped_and_the_reply_still_has_its_origin() {
        let huge = vec![json!({ "type": "section",
                               "text": { "type": "mrkdwn", "text": "x".repeat(PRIVATE_METADATA_CHARS) } })];
        let metadata = origin(huge).to_metadata();
        assert!(
            metadata.chars().count() <= PRIVATE_METADATA_CHARS,
            "bug: Slack refuses a view whose private_metadata passes its bound"
        );
        let carried: ReplyOrigin = serde_json::from_str(&metadata).unwrap();
        assert!(carried.card.is_empty(), "the card is what gives way");
        assert_eq!(
            carried.channel_id, "C0FEEDBACK",
            "the room is never dropped"
        );
    }

    #[test]
    fn a_reply_keeps_the_buttons_and_replaces_the_previous_reply_line() {
        let first = replied_card(&a_card(), "@phil", "Fixed in tonight's build.").unwrap();
        let second = replied_card(first.as_array().unwrap(), "@jf", "Thanks <@U1> & co").unwrap();
        let blocks = second.as_array().unwrap();
        assert_eq!(blocks.len(), 3, "the notice, the buttons, one reply line");
        assert_eq!(
            blocks[1]["type"], "actions",
            "the buttons stay for a follow-up"
        );
        assert_eq!(
            blocks[2]["block_id"], REPLIED_BLOCK_ID,
            "bug: the reply line is the one the next reply replaces"
        );
        assert_eq!(
            blocks[2]["elements"][0]["text"],
            ":leftwards_arrow_with_hook: @jf replied: Thanks &lt;@U1&gt; &amp; co",
            "the team's text is escaped, never a live mention"
        );
        assert_eq!(
            replied_card(&[], "@phil", "hi"),
            None,
            "no card, nothing redrawn"
        );
    }

    #[test]
    fn a_reply_settles_the_line_an_earlier_refused_tap_left() {
        let mut refused = a_card();
        refused.push(json!({ "type": "context", "block_id": "ops-refusal",
                             "elements": [{ "type": "mrkdwn", "text": ":no_entry: @phil: expired" }] }));
        let card = replied_card(&refused, "@phil", "Fixed tonight.").unwrap();
        let block_ids: Vec<&str> = card
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|block| block["block_id"].as_str())
            .collect();
        assert_eq!(
            block_ids,
            [
                "notify-feedback.recorded",
                "notify-actions-feedback.recorded",
                REPLIED_BLOCK_ID
            ],
            "bug: once a reply lands, the earlier refusal under the buttons is stale"
        );
    }

    #[test]
    fn a_submission_reads_back_the_reply_its_sender_and_its_origin() {
        let from = origin(a_card());
        let payload = json!({
            "type": "view_submission",
            "user": { "id": "U0PHIL", "username": "phil" },
            "view": {
                "callback_id": REPLY_CALLBACK_ID,
                "private_metadata": from.to_metadata(),
                "state": { "values": { "feedback_reply": { "body": {
                    "type": "plain_text_input", "value": "Fixed tonight." } } } }
            }
        });
        let submission = parse_reply_submission(&payload).unwrap();
        assert_eq!(
            submission.slack_user_id, "U0PHIL",
            "bug: the reply keeps who sent it"
        );
        assert_eq!(
            submission.slack_username, "phil",
            "bug: the card names the replier by handle"
        );
        assert_eq!(
            submission.reply, "Fixed tonight.",
            "bug: the reply is read from its field"
        );
        assert_eq!(
            submission.origin, from,
            "bug: the room and card come back as the modal carried them"
        );

        let mut other = payload;
        other["view"]["callback_id"] = json!("some_other_modal");
        assert!(
            parse_reply_submission(&other).is_err(),
            "another view is not a reply"
        );
    }

    /// An athlete with an agent reply in a conversation, rated down with a
    /// reason, and a message in their feedback thread.
    struct Fixture {
        tenant_id: TenantId,
        user_id: Uuid,
        thread_message: Uuid,
        rating: Uuid,
        conversation_id: String,
        rated_message_id: String,
    }

    async fn fixture(repos: &RepositoryRegistry) -> Fixture {
        let athlete = create_test_user(&format!("{}@feedback.test", Uuid::new_v4()), None);
        let user_id = repos.users.create(&athlete).await.expect("athlete row");
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());
        let owner = user_id.to_string();
        let conversation = repos
            .chat
            .create_conversation(&owner, tenant_id, "Volume", "model", None, None)
            .await
            .expect("conversation");
        let reply = repos
            .chat
            .add_message(&AddMessageParams {
                tenant_id,
                conversation_id: &conversation.id,
                user_id: &owner,
                role: "assistant",
                content: "You rode 9 h 40 this week.",
                token_count: None,
                finish_reason: None,
                prompt_tokens: None,
                model: None,
                content_blocks: None,
                transport_policy: TransportPolicy::AnyTransport,
                channel_type: None,
                about_user_id: None,
            })
            .await
            .expect("agent reply");
        let rating = repos
            .chat
            .upsert_message_feedback(&UpsertMessageFeedbackParams {
                tenant_id,
                conversation_id: &conversation.id,
                message_id: &reply.id,
                user_id: &owner,
                rating: "down",
                comment: Some("Tuesday is counted twice"),
            })
            .await
            .expect("rating");
        let thread_message = repos
            .feedback_messages
            .insert_feedback_message(&NewFeedbackMessage {
                tenant_id,
                user_id,
                writer: FeedbackWriter::Athlete,
                body: "The week view skips my indoor ride.",
                screen: None,
                app_version: None,
                locale: None,
                about: None,
                image: None,
                at: Utc::now(),
            })
            .await
            .expect("thread message");
        Fixture {
            tenant_id,
            user_id,
            thread_message: Uuid::parse_str(&thread_message.id).unwrap(),
            rating: Uuid::parse_str(&rating.id).unwrap(),
            conversation_id: conversation.id,
            rated_message_id: reply.id,
        }
    }

    impl Fixture {
        fn thread(&self) -> FeedbackRef {
            FeedbackRef {
                kind: FeedbackKind::Thread,
                tenant_id: self.tenant_id.as_uuid(),
                user_id: self.user_id,
                id: self.thread_message,
            }
        }

        fn rating(&self) -> FeedbackRef {
            FeedbackRef {
                kind: FeedbackKind::Rating,
                tenant_id: self.tenant_id.as_uuid(),
                user_id: self.user_id,
                id: self.rating,
            }
        }
    }

    #[tokio::test]
    async fn a_reply_to_a_thread_message_lands_in_the_thread_as_the_teams_and_unread() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let athlete = fixture(repos).await;

        let stored = store_team_reply(repos, athlete.thread(), "U0PHIL", "  Fixed tonight.  ")
            .await
            .expect("the reply is stored");
        assert_eq!(
            stored.author,
            FeedbackAuthor::Team,
            "bug: a reply from Slack is the team's row"
        );
        assert_eq!(stored.body, "Fixed tonight.", "stored trimmed");
        assert_eq!(
            stored.about, None,
            "a thread message's reply is about no agent reply"
        );

        let thread = repos
            .feedback_messages
            .list_feedback_messages(athlete.tenant_id, athlete.user_id, 50)
            .await
            .unwrap();
        let authors: Vec<FeedbackAuthor> = thread.iter().map(|m| m.author).collect();
        assert_eq!(
            authors,
            [FeedbackAuthor::Athlete, FeedbackAuthor::Team],
            "bug: the reply follows the athlete's message in their thread"
        );
        assert_eq!(
            repos
                .feedback_messages
                .count_unread_team_replies(athlete.tenant_id, athlete.user_id)
                .await
                .unwrap(),
            1,
            "the athlete has a reply waiting"
        );
    }

    #[tokio::test]
    async fn a_reply_to_a_rating_starts_the_thread_about_the_rated_reply() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let athlete = fixture(repos).await;

        let given = given_feedback(repos, athlete.rating()).await.unwrap();
        assert_eq!(
            given.text.as_deref(),
            Some("Tuesday is counted twice"),
            "bug: a rating's reason is what the modal shows"
        );
        assert_eq!(
            given.rating.as_deref(),
            Some("down"),
            "bug: the modal says which way it was rated"
        );

        let stored = store_team_reply(
            repos,
            athlete.rating(),
            "U0PHIL",
            "Fixing the double count.",
        )
        .await
        .expect("the reply is stored");
        let about = stored.about.expect("about the rated reply");
        assert_eq!(
            about.conversation_id, athlete.conversation_id,
            "bug: the reply is about the rated reply's conversation"
        );
        assert_eq!(
            about.message_id, athlete.rated_message_id,
            "bug: the reply is about the agent reply that was rated"
        );
    }

    #[tokio::test]
    async fn a_reply_is_refused_when_empty_too_long_or_about_feedback_that_is_gone() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let athlete = fixture(repos).await;

        for body in ["   ".to_owned(), "x".repeat(4001)] {
            let refused = store_team_reply(repos, athlete.thread(), "U0PHIL", &body)
                .await
                .expect_err("refused");
            assert_eq!(refused.code, ErrorCode::InvalidInput, "{refused:?}");
        }

        // The row named, but in another tenant: never found, never written.
        let elsewhere = FeedbackRef {
            tenant_id: Uuid::new_v4(),
            ..athlete.thread()
        };
        let gone = store_team_reply(repos, elsewhere, "U0PHIL", "Hello")
            .await
            .expect_err("not found");
        assert_eq!(gone.code, ErrorCode::ResourceNotFound, "{gone:?}");

        let thread = repos
            .feedback_messages
            .list_feedback_messages(athlete.tenant_id, athlete.user_id, 50)
            .await
            .unwrap();
        assert_eq!(thread.len(), 1, "only the athlete's own message is there");
    }
}
