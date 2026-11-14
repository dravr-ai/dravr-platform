// ABOUTME: Which question a finished background read re-asks, and the one-line note when the athlete moved on
// ABOUTME: A live read answers its asking message only while it is the conversation's newest; otherwise its data just landed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_contremaitre::messaging_strings::KEY_LIVE_READ_OVERTAKEN;
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use tracing::{info, warn};
use uuid::Uuid;

use super::{Destination, ServerBackfillNotifier, REENTRY_HISTORY_LOOKBACK};
use crate::services::backfill_delivery::{ChannelDelivery, InAppDelivery};
use crate::services::messaging_ingress::block_render::RenderedReply;
use crate::services::platform_post::{PlatformPostRequest, PostSurface, PostedReply};

impl ServerBackfillNotifier {
    /// Whether the athlete has asked something newer in `conversation_id`
    /// than `asked`, the message a live read answers. `false` when no stored
    /// message asked, and when the thread cannot be read: the read then
    /// answers as it would have.
    pub(super) async fn overtaken(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        asked: Option<&str>,
    ) -> bool {
        let Some(asked) = asked else {
            return false;
        };
        match self
            .repos
            .chat
            .latest_user_message_id(conversation_id, &user_id.to_string(), tenant_id)
            .await
        {
            Ok(latest) => {
                let overtaken = latest.as_deref() != Some(asked);
                if overtaken {
                    info!(
                        event = "follow_up_overtaken",
                        "Live read landed after the athlete asked something newer; noting it instead of answering"
                    );
                }
                overtaken
            }
            Err(e) => {
                warn!(error = %e, "Backfill push: the newest message could not be read; answering the ask");
                false
            }
        }
    }

    /// Tell the conversation `ask` names, in one line, that the data its
    /// earlier question waited on has landed — without answering it again.
    pub(super) async fn post_overtaken_note(
        &self,
        destination: Destination,
        ask: (Uuid, TenantId, &str),
        locale: &str,
        activity_count: usize,
    ) {
        let (user_id, tenant_id, conversation_id) = ask;
        let note = self.strings.render(KEY_LIVE_READ_OVERTAKEN, locale, &[]);
        match destination {
            Destination::Channel(route) => {
                ChannelDelivery {
                    repos: &self.repos,
                    resolver: self.resolver.as_ref(),
                }
                .send(*route, user_id, tenant_id, RenderedReply::plain(note))
                .await;
            }
            Destination::InApp => {
                // The note names no session: nothing in it is stamped.
                InAppDelivery {
                    repos: &self.repos,
                    strings: &self.strings,
                    #[cfg(feature = "client-notifications")]
                    notifications: self.notifications.as_ref(),
                    transport_policy: TransportPolicy::AnyTransport,
                }
                .deliver(
                    user_id,
                    tenant_id,
                    conversation_id,
                    locale,
                    note,
                    activity_count,
                )
                .await;
            }
        }
    }

    /// Fetch the question to re-ask in this conversation — the athlete's
    /// message `asked` names, or else their most recent one — so the re-entry
    /// can re-ask it verbatim, carrying the same window (e.g. "2022") the
    /// agent must query against the now-warm cache.
    ///
    /// `get_recent_messages` selects the newest rows and hands them back in
    /// chronological order — the shape a prompt wants — so the question that
    /// triggered the backfill is the LAST `user` row, and the scan runs from
    /// the end. Read from the front it picks the oldest question in the
    /// lookback instead: on 2026-09-21 an athlete asked for monthly elevation
    /// totals and the completion push answered whether to ride that morning, a
    /// question from six hours earlier.
    ///
    /// A slash-command line is not a question — re-asking `/status` would answer
    /// a command nobody typed — so command rows are skipped. `None` when the
    /// read fails, there is no user turn, or `asked` is not in the lookback —
    /// the caller falls back to the list.
    async fn recent_user_question(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        asked: Option<&str>,
    ) -> Option<String> {
        let messages = self
            .repos
            .chat
            .get_recent_messages(
                conversation_id,
                &user_id.to_string(),
                tenant_id,
                REENTRY_HISTORY_LOOKBACK,
                TransportPolicy::FirstPartyOnly,
            )
            .await
            .inspect_err(|e| {
                warn!(error = %e, "Backfill push: recent-message read failed for re-entry");
            })
            .ok()?;
        messages
            .into_iter()
            .rev()
            .find(|m| {
                m.role == "user"
                    && !m.is_command_turn()
                    && !m.content.trim().is_empty()
                    && asked.is_none_or(|asked| m.id == asked)
            })
            .map(|m| m.content)
    }

    /// Try to answer in the agent's own words by re-asking the athlete's
    /// question as a platform post, now that the activity cache is warm.
    ///
    /// Loop-safe: re-asking a deep window after the cache is warmed serves
    /// inline from the durable cache (`covered == true`) and never re-spawns a
    /// backfill, so the post cannot retrigger another completion push.
    /// Returns `None` (caller falls back to the templated list) when no
    /// platform post is installed, the conversation has no re-askable
    /// question, or the post was withheld — the answer did not ground in the
    /// history, or refused — in which case nothing was stored.
    pub(super) async fn try_platform_post(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        conversation_id: &str,
        surface: PostSurface,
        locale: &str,
        asked: Option<&str>,
    ) -> Option<PostedReply> {
        let platform_post = self.platform_post.get()?;
        let prompt = self
            .recent_user_question(user_id, tenant_id, conversation_id, asked)
            .await?;
        platform_post
            .post(PlatformPostRequest {
                user_id,
                tenant_id,
                conversation_id,
                surface,
                locale,
                prompt: &prompt,
            })
            .await
    }
}
