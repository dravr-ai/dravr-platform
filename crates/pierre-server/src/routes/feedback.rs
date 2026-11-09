// ABOUTME: /api/me/feedback — the athlete's in-app feedback thread with the Dravr team, and its unread replies (carnet#1235)
// ABOUTME: No LLM answers it; the athlete's messages emit feedback.recorded, and the team replies from Slack (carnet#1392)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The in-app feedback thread.
//!
//! - `GET /api/me/feedback` — `{"messages": [...]}`, the newest
//!   [`THREAD_WINDOW`] messages, oldest first.
//! - `POST /api/me/feedback` with `{"body": "…", "screen", "app_version",
//!   "locale", "about": {"conversation_id", "message_id"}}` (all but `body`
//!   optional) — store the message and answer `201 {"message": …}`.
//! - `GET /api/me/feedback/unread` — `{"unread": n}`, the team replies the
//!   athlete has not opened, which the web app's unread dot shows.
//! - `POST /api/me/feedback/read` — mark them read, `{"marked": n}`.
//! - `GET /api/me/feedback/{id}/image` — the screenshot a message carries.
//!
//! A message may carry one screenshot, `"image": {"media_type", "data"}`
//! with `data` in standard base64: at most [`MAX_IMAGE_BYTES`] once decoded,
//! and a PNG, JPEG, WebP or GIF by its own leading bytes, whatever the
//! client declared. It is kept where the athlete's uploaded files are
//! ([`OriginalsStore::put_feedback_image`]) and shared into the feedback room
//! so the team sees it beside the card. A message with a screenshot may
//! carry no text.
//!
//! [`OriginalsStore::put_feedback_image`]: pierre_providers::originals::OriginalsStore::put_feedback_image
//!
//! `screen`, `app_version` and `locale` are what the client says about
//! itself; they are cut to a bound rather than refused, so a long route never
//! costs the athlete their message. `about` names the agent reply a message
//! is about when the thread was opened from a thumbs-down, and is checked:
//! the caller must have rated that reply in that conversation, in this
//! tenant, otherwise `404` — a thread message never points at someone else's
//! conversation.
//!
//! Nothing answers the thread automatically. The team reads each message
//! where `feedback.recorded` lands and replies from that Slack card; a reply
//! is a row whose `author` is `team`, signed "Dravr team" by the client.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{
    FeedbackAbout, FeedbackImage, FeedbackMessage, FeedbackWriter, NewFeedbackMessage,
};
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::mcp::resources::ServerContext;
use crate::routes::chat;
use crate::services::feedback_events::{self, FeedbackRecorded, FeedbackSource};

/// How many of the newest messages `GET` returns.
const THREAD_WINDOW: i64 = 200;
/// The longest message the thread accepts, in characters, from the athlete
/// here and from the team's Slack reply modal.
pub(crate) const MAX_BODY_CHARS: usize = 4000;
/// Bounds on what the client says about itself, in characters.
const MAX_SCREEN_CHARS: usize = 120;
const MAX_APP_VERSION_CHARS: usize = 64;
const MAX_LOCALE_CHARS: usize = 16;
/// The largest screenshot a message carries, in bytes once decoded.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
/// The largest request body the thread takes: a screenshot at the bound in
/// base64 (4/3 of it) with room for the text and the client's context.
const MAX_REQUEST_BYTES: usize = 7 * 1024 * 1024 + 64 * 1024;

/// The screenshot types a message may carry, each with the leading bytes that
/// mark a file of that type.
const IMAGE_SIGNATURES: [(&str, &[u8]); 3] = [
    ("image/png", b"\x89PNG\r\n\x1a\n"),
    ("image/jpeg", b"\xff\xd8\xff"),
    ("image/gif", b"GIF8"),
];

/// The agent reply a message is about, on the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AboutBody {
    conversation_id: String,
    message_id: String,
}

impl From<FeedbackAbout> for AboutBody {
    fn from(about: FeedbackAbout) -> Self {
        Self {
            conversation_id: about.conversation_id,
            message_id: about.message_id,
        }
    }
}

/// A screenshot on the wire: its declared type and its bytes in standard
/// base64.
#[derive(Debug, Deserialize)]
struct ImageBody {
    media_type: String,
    data: String,
}

/// Body of `POST /api/me/feedback`.
#[derive(Debug, Deserialize)]
struct NewMessageBody {
    #[serde(default)]
    body: String,
    #[serde(default)]
    screen: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    about: Option<AboutBody>,
    #[serde(default)]
    image: Option<ImageBody>,
}

/// One message of the thread, on the wire. `author` is `athlete` or `team`;
/// `has_image` says `GET /api/me/feedback/{id}/image` serves its screenshot.
#[derive(Debug, Serialize)]
struct MessageBody {
    id: String,
    author: &'static str,
    body: String,
    created_at: DateTime<Utc>,
    about: Option<AboutBody>,
    has_image: bool,
}

impl From<FeedbackMessage> for MessageBody {
    fn from(message: FeedbackMessage) -> Self {
        Self {
            id: message.id,
            author: message.author.as_str(),
            body: message.body,
            created_at: message.created_at,
            about: message.about.map(AboutBody::from),
            has_image: message.image.is_some(),
        }
    }
}

/// Body of `GET /api/me/feedback`.
#[derive(Debug, Serialize)]
struct ThreadBody {
    messages: Vec<MessageBody>,
}

/// Body of a `201` from `POST /api/me/feedback`.
#[derive(Debug, Serialize)]
struct CreatedBody {
    message: MessageBody,
}

/// Body of `GET /api/me/feedback/unread`.
#[derive(Debug, Serialize)]
struct UnreadBody {
    unread: i64,
}

/// Body of `POST /api/me/feedback/read`.
#[derive(Debug, Serialize)]
struct MarkedBody {
    marked: u64,
}

/// `GET /api/me/feedback`.
async fn get_thread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<ThreadBody>> {
    let auth = auth.into_inner();
    let tenant_id = chat::get_tenant_id(&auth, &resources).await?;
    let messages = resources
        .common
        .repos
        .feedback_messages
        .list_feedback_messages(tenant_id, auth.user_id, THREAD_WINDOW)
        .await?;
    Ok(Json(ThreadBody {
        messages: messages.into_iter().map(MessageBody::from).collect(),
    }))
}

/// `POST /api/me/feedback`.
///
/// The tenant is the chat routes' own resolution, so an `about` reply is
/// looked up where it was written.
async fn post_message(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(request): Json<NewMessageBody>,
) -> AppResult<(StatusCode, Json<CreatedBody>)> {
    let auth = auth.into_inner();
    let tenant_id = chat::get_tenant_id(&auth, &resources).await?;
    let user_id = auth.user_id.to_string();

    let body = request.body.trim();
    let image = request.image.as_ref().map(decode_image).transpose()?;
    if body.is_empty() && image.is_none() {
        return Err(AppError::invalid_input(
            "A feedback message cannot be empty",
        ));
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(AppError::invalid_input(format!(
            "A feedback message holds at most {MAX_BODY_CHARS} characters"
        )));
    }

    let about = match request.about {
        Some(about) => Some(rated_reply(&resources, &user_id, tenant_id, about).await?),
        None => None,
    };
    let screen = bounded(request.screen.as_deref(), MAX_SCREEN_CHARS);
    let app_version = bounded(request.app_version.as_deref(), MAX_APP_VERSION_CHARS);
    let locale = bounded(request.locale.as_deref(), MAX_LOCALE_CHARS);

    // The bytes are kept before the row that names them is written, so a row
    // never names a screenshot the blob store does not hold.
    let kept_image = match &image {
        Some(screenshot) => {
            resources
                .common
                .originals
                .put_feedback_image(
                    &tenant_id,
                    auth.user_id,
                    &screenshot.kept.sha256,
                    &screenshot.kept.media_type,
                    screenshot.bytes.clone(),
                )
                .await?;
            Some(&screenshot.kept)
        }
        None => None,
    };

    let message = resources
        .common
        .repos
        .feedback_messages
        .insert_feedback_message(&NewFeedbackMessage {
            tenant_id,
            user_id: auth.user_id,
            writer: FeedbackWriter::Athlete,
            body,
            screen: screen.as_deref(),
            app_version: app_version.as_deref(),
            locale: locale.as_deref(),
            about: about.as_ref(),
            image: kept_image,
            at: Utc::now(),
        })
        .await?;

    let agent_id = match &about {
        Some(about) => {
            feedback_events::conversation_agent(
                &resources,
                &about.conversation_id,
                &user_id,
                tenant_id,
            )
            .await
        }
        None => None,
    };
    feedback_events::emit_feedback_recorded(&FeedbackRecorded {
        tenant_id,
        user_id: &user_id,
        feedback_id: &message.id,
        source: FeedbackSource::Thread,
        surface: "app",
        rating: None,
        conversation_id: about.as_ref().map(|a| a.conversation_id.as_str()),
        message_id: about.as_ref().map(|a| a.message_id.as_str()),
        agent_id: agent_id.as_deref(),
        comment: Some(body),
        has_image: image.is_some(),
    });
    if let Some(screenshot) = image {
        feedback_events::share_feedback_image(
            &resources,
            &message.id,
            &screenshot.kept.media_type,
            screenshot.bytes,
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(CreatedBody {
            message: MessageBody::from(message),
        }),
    ))
}

/// `GET /api/me/feedback/{id}/image`: the screenshot one of the caller's
/// own messages carries, `404` for any other. The web thread reads it
/// through the API client (`feedbackApi.getImage`).
async fn get_image(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let auth = auth.into_inner();
    let tenant_id = chat::get_tenant_id(&auth, &resources).await?;
    let image = resources
        .common
        .repos
        .feedback_messages
        .get_feedback_message(tenant_id, auth.user_id, &id)
        .await?
        .and_then(|message| message.image)
        .ok_or_else(|| AppError::not_found("Screenshot not found"))?;
    let bytes = resources
        .common
        .originals
        .get_feedback_image(&tenant_id, auth.user_id, &image.sha256)
        .await?
        .ok_or_else(|| AppError::not_found("Screenshot not found"))?;
    Ok((
        [
            (header::CONTENT_TYPE, image.media_type),
            (header::CACHE_CONTROL, "private, max-age=86400".to_owned()),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        ],
        bytes,
    )
        .into_response())
}

/// A screenshot as the request carried it, checked: its bytes, and the
/// digest and type it is kept under.
struct Screenshot {
    bytes: Bytes,
    kept: FeedbackImage,
}

/// The type a screenshot's own leading bytes mark it as: a PNG, JPEG, GIF or
/// WebP (`RIFF`, a length, then `WEBP`), or none.
fn sniff_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    IMAGE_SIGNATURES
        .iter()
        .find(|(_, signature)| bytes.starts_with(signature))
        .map(|(media_type, _)| *media_type)
}

/// Decode and check a screenshot: standard base64, at most
/// [`MAX_IMAGE_BYTES`], and the type its bytes mark, which must be the one
/// the client declared.
fn decode_image(image: &ImageBody) -> AppResult<Screenshot> {
    let bytes = STANDARD
        .decode(image.data.trim())
        .map_err(|_| AppError::invalid_input("A screenshot must be standard base64"))?;
    if bytes.is_empty() {
        return Err(AppError::invalid_input("A screenshot cannot be empty"));
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(AppError::invalid_input(format!(
            "A screenshot holds at most {} MB",
            MAX_IMAGE_BYTES / (1024 * 1024)
        )));
    }
    let media_type = sniff_image_type(&bytes)
        .filter(|sniffed| *sniffed == image.media_type)
        .ok_or_else(|| {
            AppError::invalid_input("A screenshot must be a PNG, JPEG, WebP or GIF image")
        })?;
    Ok(Screenshot {
        kept: FeedbackImage {
            sha256: hex::encode(Sha256::digest(&bytes)),
            media_type: media_type.to_owned(),
        },
        bytes: Bytes::from(bytes),
    })
}

/// `GET /api/me/feedback/unread`.
async fn get_unread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<UnreadBody>> {
    let auth = auth.into_inner();
    let tenant_id = chat::get_tenant_id(&auth, &resources).await?;
    let unread = resources
        .common
        .repos
        .feedback_messages
        .count_unread_team_replies(tenant_id, auth.user_id)
        .await?;
    Ok(Json(UnreadBody { unread }))
}

/// `POST /api/me/feedback/read`: the athlete has the thread open, so every
/// team reply in it is read.
async fn mark_read(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<MarkedBody>> {
    let auth = auth.into_inner();
    let tenant_id = chat::get_tenant_id(&auth, &resources).await?;
    let marked = resources
        .common
        .repos
        .feedback_messages
        .mark_team_replies_read(tenant_id, auth.user_id, Utc::now())
        .await?;
    Ok(Json(MarkedBody { marked }))
}

/// The reply `about` names, once the caller is shown to have rated it in
/// that conversation and tenant — the one way into the thread that carries
/// a reply is the thumbs-down under it.
async fn rated_reply(
    resources: &ServerContext,
    user_id: &str,
    tenant_id: TenantId,
    about: AboutBody,
) -> AppResult<FeedbackAbout> {
    let rated = resources
        .common
        .repos
        .chat
        .get_conversation_feedback(&about.conversation_id, user_id, tenant_id)
        .await?
        .iter()
        .any(|feedback| feedback.message_id == about.message_id);
    if !rated {
        return Err(AppError::not_found("Reply not found"));
    }
    Ok(FeedbackAbout {
        conversation_id: about.conversation_id,
        message_id: about.message_id,
    })
}

/// A client-supplied label, trimmed and cut to `max` characters; blank reads
/// as absent.
fn bounded(value: Option<&str>, max: usize) -> Option<String> {
    let value = value?.trim();
    (!value.is_empty()).then(|| value.chars().take(max).collect())
}

/// Mount-helper for the feedback thread endpoints.
#[derive(Debug)]
pub struct FeedbackRoutes;

impl FeedbackRoutes {
    /// User-facing routes. Require a valid session; no admin gate.
    pub fn routes(resources: Arc<ServerContext>) -> Router {
        Router::new()
            .route("/api/me/feedback", get(get_thread).post(post_message))
            .route("/api/me/feedback/unread", get(get_unread))
            .route("/api/me/feedback/read", post(mark_read))
            .route("/api/me/feedback/{id}/image", get(get_image))
            .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
            .with_state(resources)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_is_trimmed_and_cut_on_characters() {
        assert_eq!(
            bounded(Some("  home  "), 10).as_deref(),
            Some("home"),
            "bug: a label must lose its surrounding blanks"
        );
        assert_eq!(
            bounded(Some("ééééé"), 3).as_deref(),
            Some("ééé"),
            "bug: a label must be cut on characters, never inside one"
        );
    }

    fn screenshot(media_type: &str, bytes: &[u8]) -> ImageBody {
        ImageBody {
            media_type: media_type.to_owned(),
            data: STANDARD.encode(bytes),
        }
    }

    #[test]
    fn a_screenshot_is_the_type_its_own_bytes_mark() {
        let png = b"\x89PNG\r\n\x1a\nrest of the image";
        let kept = decode_image(&screenshot("image/png", png)).expect("a PNG is kept");
        assert_eq!(
            kept.kept.media_type, "image/png",
            "bug: a PNG is kept as a PNG"
        );
        assert_eq!(
            kept.bytes.as_ref(),
            png,
            "bug: the bytes are kept as the client sent them"
        );
        assert_eq!(
            kept.kept.sha256,
            hex::encode(Sha256::digest(png)),
            "bug: a screenshot is named by the digest of its bytes"
        );

        let webp = b"RIFF\x10\x00\x00\x00WEBPVP8 ";
        assert_eq!(
            sniff_image_type(webp),
            Some("image/webp"),
            "bug: RIFF, a length, then WEBP marks a WebP"
        );
        assert_eq!(
            sniff_image_type(b"\xff\xd8\xff\xe0"),
            Some("image/jpeg"),
            "bug: FF D8 FF marks a JPEG"
        );
        assert_eq!(
            sniff_image_type(b"GIF89a"),
            Some("image/gif"),
            "bug: GIF8 marks a GIF"
        );
        assert_eq!(
            sniff_image_type(b"<svg xmlns="),
            None,
            "bug: an SVG can carry script"
        );
    }

    #[test]
    fn a_screenshot_declared_as_another_type_or_too_large_is_refused() {
        let png = b"\x89PNG\r\n\x1a\n";
        assert!(
            decode_image(&screenshot("image/gif", png)).is_err(),
            "bug: a PNG declared as a GIF must not be kept as either"
        );
        assert!(
            decode_image(&screenshot("image/png", b"%PDF-1.7")).is_err(),
            "bug: a PDF declared as a PNG must be refused"
        );
        assert!(
            decode_image(&screenshot("image/png", b"")).is_err(),
            "bug: an empty screenshot must be refused"
        );
        let not_base64 = ImageBody {
            media_type: "image/png".to_owned(),
            data: "not base64 at all!".to_owned(),
        };
        assert!(
            decode_image(&not_base64).is_err(),
            "bug: data that is not standard base64 must be refused"
        );

        let mut at_bound = png.to_vec();
        at_bound.resize(MAX_IMAGE_BYTES, 0);
        assert!(
            decode_image(&screenshot("image/png", &at_bound)).is_ok(),
            "bug: a screenshot exactly at the bound is kept"
        );
        at_bound.push(0);
        assert!(
            decode_image(&screenshot("image/png", &at_bound)).is_err(),
            "bug: a byte past the bound is refused"
        );
    }

    #[test]
    fn a_blank_label_reads_as_absent() {
        assert_eq!(
            bounded(Some("   "), 10),
            None,
            "bug: a blank label must read as no label"
        );
        assert_eq!(bounded(None, 10), None, "bug: no label must stay none");
    }
}
