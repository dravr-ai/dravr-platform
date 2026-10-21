// ABOUTME: Where a post the coach makes on its own lands — the athlete's own thread, and the DM they last wrote from
// ABOUTME: Shared by the first-sync post and the check-in: destinations decided before any turn, one turn per destination
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The destinations of a platform post (carnet#831).
//!
//! Decided before any turn runs, so no tokens are spent on a post nobody can
//! receive:
//!
//! - **In the app**: the athlete's personal thread ([`personal_thread`]); one
//!   is forged for the post when they have none, and removed again when
//!   nothing was posted into it.
//! - **On a DM**: the messaging DM they last wrote from, among those whose
//!   channel may be written to unprompted right now
//!   ([`channel_allows_proactive`] — Meta's 24-hour window on `WhatsApp` and
//!   Messenger). A DM whose window is closed is *held*: no turn runs for it,
//!   and the caller says so.
//!
//! Each destination is its own turn, laid out for its surface and stored in
//! its own conversation, so a reply in either thread reads the post as
//! history. A DM that writes into the personal thread (carnet#840) is one
//! destination, not two: the post made there is the one the app shows.

use chrono::{DateTime, Utc};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_services::locale::{resolve_channel_locale, resolve_user_locale};
use pierre_services::messaging_broadcast::resolve_linked_targets;
use pierre_services::personal_thread::{
    find_personal_thread, resolve_personal_thread, PersonalForge,
};
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::adapter_resolver::AdapterResolver;
use crate::services::backfill_delivery::{ChannelDelivery, ResolvedRoute};
use crate::services::commitment_reporter::channel_allows_proactive;
use crate::services::messaging_ingress::block_render::render_reply;
use crate::services::messaging_ingress::fresh_title_fallback;
use crate::services::messaging_ingress::surface::messaging_render_profile;
use crate::services::platform_post::{PlatformPost, PlatformPostRequest, PostSurface};

/// The athlete a post is for.
#[derive(Debug, Clone, Copy)]
pub struct PostAthlete {
    /// The athlete the turn runs as.
    pub user_id: Uuid,
    /// The athlete's own tenant, where their threads and DM sessions live.
    pub tenant_id: TenantId,
}

/// The linked messaging DM a post can be written into right now.
pub struct OpenDm {
    /// The messaging session the DM belongs to.
    session_id: String,
    /// The Pierre conversation the DM's turns are stored in.
    conversation_id: String,
    /// The DM's channel.
    pub channel_type: ChannelType,
    /// The athlete's id on that channel, which addresses the DM.
    recipient: String,
    /// The tenant whose bot holds the link, and so the channel config.
    channel_tenant_id: TenantId,
    /// When the athlete last wrote on the DM.
    last_inbound: Option<DateTime<Utc>>,
}

impl OpenDm {
    /// Whether the DM writes into the athlete's personal thread (carnet#840),
    /// so a post made there is the in-app post as well: the app shows that
    /// thread, and a second post into it would say the same thing twice.
    pub async fn shares_personal_thread(
        &self,
        resources: &ServerContext,
        athlete: PostAthlete,
    ) -> bool {
        match find_personal_thread(
            &resources.common.repos,
            &athlete.user_id.to_string(),
            athlete.tenant_id,
        )
        .await
        {
            Ok(Some(thread)) => thread.id == self.conversation_id,
            Ok(None) => false,
            Err(e) => {
                warn!(error = %e, "Platform post: the athlete's personal thread could not be read");
                false
            }
        }
    }
}

/// The athlete's linked DMs as a post sees them now.
pub struct LinkedDms {
    /// The DM they last wrote from among those that may be written to
    /// unprompted, if any.
    pub open: Option<OpenDm>,
    /// The channels whose DM may not be written to unprompted now: the window
    /// closed, so a post there is held.
    pub held: Vec<ChannelType>,
}

/// The athlete's in-app thread a post lands in.
struct InAppThread {
    /// The conversation id.
    id: String,
    /// Whether it was forged for this post, and so is empty without it.
    forged: bool,
}

/// Post into the athlete's own in-app thread, with the instruction `prompt`
/// gives for their locale.
///
/// `None` when nothing was attempted — no instruction in that locale, or no
/// thread could be read or forged; otherwise whether the reply was stored.
pub async fn post_in_app(
    resources: &ServerContext,
    post: &dyn PlatformPost,
    athlete: PostAthlete,
    prompt: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Option<bool> {
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), athlete.user_id).await;
    let prompt = prompt(&locale)?;
    let thread = personal_thread(resources, athlete).await?;
    let posted = post
        .post(PlatformPostRequest {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id,
            conversation_id: &thread.id,
            surface: PostSurface::InApp,
            locale: &locale,
            prompt: &prompt,
        })
        .await
        .is_some();
    if !posted && thread.forged {
        // The thread was opened to hold this post alone; left behind it is an
        // empty row in the athlete's list.
        if let Err(e) = resources
            .common
            .repos
            .chat
            .delete_conversation(&thread.id, &athlete.user_id.to_string(), athlete.tenant_id)
            .await
        {
            warn!(error = %e, "Platform post: removing the thread forged for it failed");
        }
    }
    Some(posted)
}

/// Post into `dm` and send the reply there, with the instruction `prompt`
/// gives for the DM's locale.
///
/// `None` when nothing was attempted — no instruction in that locale;
/// otherwise whether the reply was stored in the DM's conversation. A stored
/// reply whose send failed is queued for the outbound retry worker.
pub async fn post_to_dm(
    resources: &ServerContext,
    post: &dyn PlatformPost,
    resolver: &dyn AdapterResolver,
    athlete: PostAthlete,
    dm: OpenDm,
    prompt: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Option<bool> {
    let channel_str = dm.channel_type.to_string();
    let locale = resolve_channel_locale(
        resources.common.repos.messaging.as_ref(),
        resources.common.repos.users.as_ref(),
        athlete.tenant_id,
        &channel_str,
        &dm.recipient,
        Some(athlete.user_id),
    )
    .await;
    let prompt = prompt(&locale)?;
    let Some(reply) = post
        .post(PlatformPostRequest {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id,
            conversation_id: &dm.conversation_id,
            surface: PostSurface::Channel(dm.channel_type),
            locale: &locale,
            prompt: &prompt,
        })
        .await
    else {
        return Some(false);
    };
    // The same layout a live reply takes: the prose as the channel shows typed
    // text, then each chart as its own message.
    let rendered = render_reply(
        &messaging_render_profile(dm.channel_type, &locale).render,
        &reply.assistant,
        &resources.mcp.messaging_strings_registry,
        &locale,
    );
    let delivery = ChannelDelivery {
        repos: &resources.common.repos,
        resolver,
    };
    if delivery
        .send(
            ResolvedRoute {
                session_id: dm.session_id,
                channel_str: channel_str.clone(),
                channel_type: dm.channel_type,
                recipient: dm.recipient.clone(),
                channel_user_id: dm.recipient,
                channel_tenant_id: dm.channel_tenant_id,
            },
            athlete.user_id,
            athlete.tenant_id,
            rendered,
        )
        .await
    {
        info!(channel = %channel_str, "Platform post sent on the athlete's DM");
    }
    Some(true)
}

/// The in-app thread a post lands in: the athlete's personal thread
/// (carnet#840), adopted or forged for the post when none is designated yet.
async fn personal_thread(resources: &ServerContext, athlete: PostAthlete) -> Option<InAppThread> {
    let user = athlete.user_id.to_string();
    let title_fallback = fresh_title_fallback(resources, &user).await;
    match resolve_personal_thread(
        &resources.common.repos,
        &user,
        athlete.tenant_id,
        PersonalForge {
            title_fallback: &title_fallback,
            // In the app the pillars walk is offered, never started for the
            // athlete (carnet#1099): Finish your profile is a link they take.
            // A DM post lands in the DM's own conversation, not forged here.
            guided_flow: false,
            llm_chain: &resources.common.config.llm_chain,
        },
    )
    .await
    {
        Ok(thread) => Some(InAppThread {
            id: thread.conversation.id,
            forged: thread.forged,
        }),
        Err(e) => {
            warn!(error = %e, "Platform post: the athlete's personal thread could not be resolved");
            None
        }
    }
}

/// The athlete's linked DMs at `now`: the one they last wrote from among
/// those whose channel may be written to unprompted, and the channels held by
/// a closed window.
pub async fn linked_dms(
    resources: &ServerContext,
    athlete: PostAthlete,
    now: DateTime<Utc>,
) -> LinkedDms {
    let messaging = resources.common.repos.messaging.as_ref();
    let mut open: Vec<OpenDm> = Vec::new();
    let mut held: Vec<ChannelType> = Vec::new();
    for target in resolve_linked_targets(messaging, athlete.user_id).await {
        let channel_str = target.channel_type.to_string();
        // A DM's session is stored under the athlete's own tenant, never the
        // bot's; `None` is the DM's empty chat id.
        let session = match messaging
            .get_session_by_channel_identity(
                athlete.tenant_id,
                &channel_str,
                &target.recipient_id,
                None,
            )
            .await
        {
            Ok(Some(session)) => session,
            Ok(None) => continue,
            Err(e) => {
                warn!(error = %e, channel = %channel_str, "Platform post: DM session lookup failed");
                continue;
            }
        };
        let last_inbound = session
            .get("last_message_at")
            .and_then(Value::as_str)
            .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
            .map(|at| at.with_timezone(&Utc));
        if !channel_allows_proactive(target.channel_type, last_inbound, now) {
            info!(channel = %channel_str, "Platform post: the DM's window is closed");
            held.push(target.channel_type);
            continue;
        }
        let (Some(session_id), Some(conversation_id)) = (
            session.get("id").and_then(Value::as_str),
            session
                .get("pierre_conversation_id")
                .and_then(Value::as_str),
        ) else {
            continue;
        };
        open.push(OpenDm {
            session_id: session_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            channel_type: target.channel_type,
            recipient: target.recipient_id,
            channel_tenant_id: target.tenant_id,
            last_inbound,
        });
    }
    LinkedDms {
        open: open.into_iter().max_by_key(|dm| dm.last_inbound),
        held,
    }
}
