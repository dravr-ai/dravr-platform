// ABOUTME: GET and POST /api/me/feedback — the athlete's in-app feedback thread with the Dravr team (carnet#1235)
// ABOUTME: No LLM answers it; each message is stored per tenant and athlete and emits feedback.recorded for the team
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
//! where `feedback.recorded` lands.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::repositories::{FeedbackAbout, FeedbackMessage, NewFeedbackMessage};
use pierre_middleware::extractors::AuthenticatedUser;
use serde::{Deserialize, Serialize};

use crate::mcp::resources::ServerContext;
use crate::routes::chat;
use crate::services::feedback_events::{self, FeedbackRecorded, FeedbackSource};

/// How many of the newest messages `GET` returns.
const THREAD_WINDOW: i64 = 200;
/// The longest message the thread accepts, in characters.
const MAX_BODY_CHARS: usize = 4000;
/// Bounds on what the client says about itself, in characters.
const MAX_SCREEN_CHARS: usize = 120;
const MAX_APP_VERSION_CHARS: usize = 64;
const MAX_LOCALE_CHARS: usize = 16;

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

/// Body of `POST /api/me/feedback`.
#[derive(Debug, Deserialize)]
struct NewMessageBody {
    body: String,
    #[serde(default)]
    screen: Option<String>,
    #[serde(default)]
    app_version: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    about: Option<AboutBody>,
}

/// One message of the thread, on the wire.
#[derive(Debug, Serialize)]
struct MessageBody {
    id: String,
    body: String,
    created_at: DateTime<Utc>,
    about: Option<AboutBody>,
}

impl From<FeedbackMessage> for MessageBody {
    fn from(message: FeedbackMessage) -> Self {
        Self {
            id: message.id,
            body: message.body,
            created_at: message.created_at,
            about: message.about.map(AboutBody::from),
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
    if body.is_empty() {
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

    let message = resources
        .common
        .repos
        .feedback_messages
        .insert_feedback_message(&NewFeedbackMessage {
            tenant_id,
            user_id: auth.user_id,
            body,
            screen: screen.as_deref(),
            app_version: app_version.as_deref(),
            locale: locale.as_deref(),
            about: about.as_ref(),
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
        source: FeedbackSource::Thread,
        surface: "app",
        rating: None,
        conversation_id: about.as_ref().map(|a| a.conversation_id.as_str()),
        message_id: about.as_ref().map(|a| a.message_id.as_str()),
        agent_id: agent_id.as_deref(),
        comment: Some(body),
    });

    Ok((
        StatusCode::CREATED,
        Json(CreatedBody {
            message: MessageBody::from(message),
        }),
    ))
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
