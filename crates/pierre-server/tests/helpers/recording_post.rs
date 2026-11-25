// ABOUTME: A PlatformPost that records each post it is asked for and answers as a grounded or a withheld one
// ABOUTME: Included via `#[path] mod recording_post;` by the suites that pin where a coach post lands (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use std::sync::Mutex;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_chat_pipeline::{AssistantTurn, ReplyBlock};
use pierre_core::models::{MessageRecord, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_mcp_server::services::platform_post::{
    PlatformPost, PlatformPostRequest, PostAccess, PostSurface, PostedReply,
};
use uuid::Uuid;

/// What the faked post answers when it is posted.
pub(crate) const REPLY: &str = "Your first WHOOP week: recovery held around 70%.";

/// The agent the faked post answers as, as the pipeline names the agent a
/// thread is bound to: what a push of the landed post names.
pub(crate) const ANSWERED_BY: &str = "Coach";

/// One post a caller asked for.
#[derive(Debug, Clone)]
pub(crate) struct Asked {
    pub user_id: Uuid,
    pub tenant_id: TenantId,
    pub conversation_id: String,
    pub surface: PostSurface,
    pub prompt: String,
    pub access: PostAccess,
    /// The appendix a post carried with the findings the platform supplied.
    pub appendix: Option<String>,
}

/// A platform post that records what it was asked to post and answers as a
/// grounded post (`posts`) or a withheld one. Mocked: whether a real reply is
/// posted is the pipeline's to decide, and `platform_post_test` pins that.
pub(crate) struct RecordingPost {
    posts: bool,
    asked: Mutex<Vec<Asked>>,
}

impl RecordingPost {
    pub(crate) fn posting(posts: bool) -> Self {
        Self {
            posts,
            asked: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn asked(&self) -> Vec<Asked> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl PlatformPost for RecordingPost {
    async fn post(&self, request: PlatformPostRequest<'_>) -> Option<PostedReply> {
        self.asked.lock().unwrap().push(Asked {
            user_id: request.user_id,
            tenant_id: request.tenant_id,
            conversation_id: request.conversation_id.to_owned(),
            surface: request.surface,
            prompt: request.prompt.to_owned(),
            access: request.access,
            appendix: request
                .findings
                .map(|findings| findings.appendix.to_owned()),
        });
        self.posts.then(|| PostedReply {
            answered_by: Some(ANSWERED_BY.to_owned()),
            assistant: AssistantTurn {
                message: MessageRecord {
                    id: Uuid::new_v4().to_string(),
                    conversation_id: request.conversation_id.to_owned(),
                    role: "assistant".to_owned(),
                    content: REPLY.to_owned(),
                    token_count: None,
                    prompt_tokens: None,
                    model: Some("mock-model".to_owned()),
                    finish_reason: Some("stop".to_owned()),
                    content_blocks: None,
                    transport_policy: TransportPolicy::AnyTransport,
                    created_at: Timestamp::now().to_rfc3339(),
                    channel_type: None,
                    about_user_id: None,
                    turn_attempted_write: None,
                    ai_consent_sources: Vec::new(),
                },
                blocks: vec![ReplyBlock::Prose {
                    text: REPLY.to_owned(),
                }],
                finish_reason: Some("stop".to_owned()),
            },
        })
    }
}
