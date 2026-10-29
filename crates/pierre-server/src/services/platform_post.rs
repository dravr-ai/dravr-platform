// ABOUTME: A verified coach post the platform makes on its own initiative, run as one chat-pipeline turn
// ABOUTME: The PlatformPost seam, its pipeline implementation, and its post-Arc install on the context
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Server-initiated coach posts.
//!
//! The coach sometimes speaks without the athlete starting the turn: a
//! completed backfill answers the athlete's earlier question in persona, the
//! coach speaks first once an athlete's data lands, and it checks in on the
//! cadence the athlete chose (carnet#831). Every such post is one ordinary
//! chat-pipeline turn the platform composed ([`TurnOrigin::Platform`]), so
//! the claim verifier, the transport policy,
//! the usage caps and the athlete's own LLM key apply exactly as on a turn they
//! sent, and a chart the agent draws on a messaging surface leaves as an image
//! rather than as its positional marker.
//!
//! The prompt is never written to the athlete's thread. The reply is stored in
//! the conversation it was posted into only when it grounded in the athlete's
//! data and did not refuse
//! ([`pierre_chat_pipeline::stages::platform_grounding`]); otherwise nothing is
//! written and [`PlatformPost::post`] answers `None`. A stored reply leaves the
//! athlete's read marker where it was, so it badges as unread, and never fans
//! out to a group room.
//!
//! Delivering the reply anywhere beyond the conversation — out through a
//! messaging channel — is the caller's: the turn's blocks come back laid out
//! for the surface the post was made for.

use std::fmt;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use pierre_chat_pipeline::{
    AssistantTurn, CommandPersistence, InputSource, PipelineHooks, ScenePublisher, ServedTurn,
    SurfaceId, SurfaceProfile, SurfaceRequest, TurnOrigin, TurnRequest,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{ConversationTurnId, TenantId, CHANNEL_TYPE_WEB};
use pierre_core::transport::Transport;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::build_messaging_profile;
use crate::services::messaging_ingress::scene_publisher::{
    athlete_color_scheme, MessagingScenePublisher,
};

/// The surface a platform post is laid out for: the conversation it lands in
/// is read there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostSurface {
    /// A first-party (web or mobile) conversation, read by fetching it.
    InApp,
    /// A messaging conversation, delivered through that channel's adapter.
    Channel(ChannelType),
}

impl PostSurface {
    /// The surface's name as analytics reports a post's destination.
    #[must_use]
    pub fn destination(self) -> String {
        match self {
            Self::InApp => "in_app".to_owned(),
            Self::Channel(channel) => channel.to_string(),
        }
    }
}

/// One platform post: who it is for, where it lands, and the instruction it
/// answers. Borrowed so the request allocates nothing.
pub struct PlatformPostRequest<'a> {
    /// The athlete the turn runs as.
    pub user_id: Uuid,
    /// The athlete's own tenant: their conversation, messages, activity cache
    /// and usage counters all live under it.
    pub tenant_id: TenantId,
    /// The conversation the reply is stored in.
    pub conversation_id: &'a str,
    /// The surface that conversation is read on.
    pub surface: PostSurface,
    /// BCP-47 short locale the post is written in.
    pub locale: &'a str,
    /// The instruction the turn answers — the athlete's own earlier question
    /// re-asked, or a prompt the platform composed. Never stored.
    pub prompt: &'a str,
}

// The prompt can be the athlete's own question re-asked: only its size is printed.
impl fmt::Debug for PlatformPostRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlatformPostRequest")
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .field("conversation_id", &self.conversation_id)
            .field("surface", &self.surface)
            .field("locale", &self.locale)
            .field("prompt_len", &self.prompt.len())
            .finish()
    }
}

/// A platform post that was stored in its conversation.
pub struct PostedReply {
    /// The turn's blocks, as the pipeline laid them out for the post's
    /// surface, and the stored assistant record.
    pub assistant: AssistantTurn,
}

// The assistant record holds the reply text: only ids and sizes are printed.
impl fmt::Debug for PostedReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PostedReply")
            .field("message_id", &self.assistant.message.id)
            .field("content_len", &self.assistant.message.content.len())
            .field("blocks", &self.assistant.blocks.len())
            .finish_non_exhaustive()
    }
}

/// Makes a platform post through the chat pipeline.
///
/// A trait so the callers' "post, else fall back" branches are testable
/// against a fake that returns a canned reply without booting the pipeline.
#[async_trait]
pub trait PlatformPost: Send + Sync {
    /// Run the post. `Some` when the reply was stored in the conversation;
    /// `None` when it was withheld — ungrounded, or a refusal — or when the
    /// turn could not run. Implementations log their own failures.
    async fn post(&self, request: PlatformPostRequest<'_>) -> Option<PostedReply>;
}

/// Production [`PlatformPost`]: drives `pierre_chat_pipeline::execute`
/// through the composition-root [`ServerContext`].
///
/// Holds a [`Weak`] handle to break the DI-graph cycle — the context owns the
/// slot this post is installed into, so a strong reference back to the context
/// would leak the whole container. On a missed upgrade (shutdown in flight)
/// nothing is posted.
#[derive(Debug)]
pub struct PipelinePlatformPost {
    /// Weak handle to the composition root; upgraded per call.
    ctx: Weak<ServerContext>,
}

impl PipelinePlatformPost {
    /// Wrap a weak handle to the composition root.
    #[must_use]
    pub const fn new(ctx: Weak<ServerContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl PlatformPost for PipelinePlatformPost {
    async fn post(&self, request: PlatformPostRequest<'_>) -> Option<PostedReply> {
        let resources = self.ctx.upgrade()?;
        let (profile, channel_slug) = match request.surface {
            PostSurface::InApp => (
                SurfaceProfile::resolve(&SurfaceRequest {
                    surface: SurfaceId::Web,
                    locale: request.locale.to_owned(),
                    transport: None,
                    prose_contract: None,
                }),
                CHANNEL_TYPE_WEB.to_owned(),
            ),
            PostSurface::Channel(channel) => (
                build_messaging_profile(&resources, channel, request.locale.to_owned()),
                channel.to_string(),
            ),
        };
        // Charts: a channel cannot draw a spec, so the pipeline asks the
        // publisher for a signed image URL per block once the assistant row is
        // durable — the same seam a live messaging reply wires. The app draws
        // specs itself and takes none.
        let scene_publisher = match request.surface {
            PostSurface::Channel(_) => Some(MessagingScenePublisher::new(
                Arc::clone(&resources),
                profile.render,
                athlete_color_scheme(&resources, request.user_id).await,
            )),
            PostSurface::InApp => None,
        };
        let ctx = resources.chat_pipeline_context();
        let turn = TurnRequest {
            // The platform composed this prompt; it is never written to the
            // athlete's transcript as though they had sent it — carnet#246.
            origin: TurnOrigin::Platform,
            conversation_id: request.conversation_id.to_owned(),
            user_id: request.user_id,
            conversation_tenant_id: request.tenant_id,
            tool_tenant_id: request.tenant_id,
            content: request.prompt.to_owned(),
            // Not the athlete's input at all: analytics reports a platform
            // turn's origin as `platform`, whatever this says.
            input_source: InputSource::Typed,
            // Fresh correlation id — this is a new (proactive) turn.
            turn_id: ConversationTurnId::new(),
            // A post lands in the athlete's own conversation, never a room.
            ambient_context: None,
            quote: None,
            channel_type: &channel_slug,
            // The platform answers on its own initiative, into the athlete's
            // own surface.
            transport: Transport::PlatformJob,
            is_direct_message: true,
            // The surface's own answers. The prompt is platform-authored and
            // never a command, so these only keep the surface's contract
            // intact.
            ambient_group_fallback: matches!(request.surface, PostSurface::Channel(_)),
            command_persistence: CommandPersistence::Always,
            sender_id: None,
            // No AG-UI wiring: this is a detached background turn, not a live
            // request with a status placeholder to edit.
            hooks: PipelineHooks {
                scene_publisher: scene_publisher
                    .as_ref()
                    .map(|publisher| -> &dyn ScenePublisher { publisher }),
                ..PipelineHooks::none()
            },
        };
        // Through the same turn service a live turn takes, so the post spends
        // the athlete's budget, honours their BYO LLM key and is refused by the
        // same caps — a proactive post is a real model call, and a ladder of
        // its own is how one stops being any of those things.
        match pierre_chat_pipeline::execute(&ctx, turn, &profile).await {
            Ok(ServedTurn::Pipeline(envelope)) if envelope.posted => Some(PostedReply {
                assistant: envelope.assistant,
            }),
            Ok(ServedTurn::Pipeline(_)) => {
                info!(
                    destination = %request.surface.destination(),
                    "Platform post withheld: the reply did not ground in the athlete's data"
                );
                None
            }
            // The prompt is platform-authored and never a command.
            Ok(ServedTurn::Command { .. }) => None,
            Err(e) => {
                warn!(error = %e, "Platform post: the pipeline turn failed");
                None
            }
        }
    }
}

/// Install the production [`PipelinePlatformPost`] on the context's slot, now
/// that the composition-root `Arc<ServerContext>` exists.
///
/// Called once from the binary's post-`Arc` wiring: the backfill notifier is
/// built inside `ServerContext::new` — before the `Arc` — so the post, which
/// needs the `Arc`, is plumbed in afterward through the shared
/// [`std::sync::OnceLock`] slot. A second call is a startup logic bug, logged
/// not fatal.
pub fn install_platform_post(resources: &Arc<ServerContext>) -> Arc<dyn PlatformPost> {
    let post: Arc<dyn PlatformPost> =
        Arc::new(PipelinePlatformPost::new(Arc::downgrade(resources)));
    if resources.mcp.platform_post.set(Arc::clone(&post)).is_err() {
        warn!("Platform post handle already installed; skipping");
    }
    post
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_chat_pipeline::ReplyBlock;
    use pierre_core::models::MessageRecord;
    use pierre_core::transport::TransportPolicy;

    const SENTINEL: &str = "sentinel-athlete-words-4b2d";

    #[test]
    fn a_post_request_prints_the_size_of_its_prompt_never_the_prompt() {
        let prompt = format!("how did my {SENTINEL} long run go?");
        let request = PlatformPostRequest {
            user_id: Uuid::nil(),
            tenant_id: TenantId::from_uuid(Uuid::nil()),
            conversation_id: "conv-1",
            surface: PostSurface::InApp,
            locale: "en",
            prompt: &prompt,
        };
        let printed = format!("{request:?} {request:#?}");
        assert!(!printed.contains(SENTINEL), "the prompt leaked: {printed}");
        assert!(
            printed.contains(&format!("prompt_len: {}", prompt.len())),
            "the prompt's size is printed: {printed}"
        );
    }

    #[test]
    fn a_posted_reply_prints_its_sizes_never_the_reply_text() {
        let reply = PostedReply {
            assistant: AssistantTurn {
                message: MessageRecord {
                    id: "msg-asst".to_owned(),
                    conversation_id: "conv-1".to_owned(),
                    role: "assistant".to_owned(),
                    content: SENTINEL.to_owned(),
                    token_count: None,
                    prompt_tokens: None,
                    model: None,
                    finish_reason: None,
                    content_blocks: None,
                    created_at: "2026-09-22T20:22:00Z".to_owned(),
                    transport_policy: TransportPolicy::AnyTransport,
                    channel_type: None,
                    about_user_id: None,
                },
                blocks: vec![ReplyBlock::Prose {
                    text: SENTINEL.to_owned(),
                }],
                finish_reason: None,
            },
        };
        let printed = format!("{reply:?} {reply:#?}");
        assert!(
            !printed.contains(SENTINEL),
            "the reply text leaked: {printed}"
        );
        assert!(
            printed.contains("msg-asst"),
            "the message id is printed: {printed}"
        );
    }
}
