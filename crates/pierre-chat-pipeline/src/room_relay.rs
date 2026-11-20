// ABOUTME: Web and mobile turns in a group connected to a group chat are relayed into that chat (carnet#1683, D2)
// ABOUTME: A member's post as « Name: … », then the agent's reply; only what the room may read, and never back where it came from

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The web half of a connected room.
//!
//! A group connected to a Telegram group chat from the web app has one room on
//! two surfaces. The chat's messages reach the web room through the group
//! transcript as soon as the chat is connected. This module carries the other
//! direction: a member's post in the app's group thread is posted into the
//! chat by the bot, attributed (« Alice: … »), and the agent's reply to it
//! follows.
//!
//! Three rules hold it:
//!
//! - **Each message crosses once.** Only a turn that arrived on Dravr's own
//!   apps ([`SurfaceId::Web`], [`SurfaceId::Mobile`]) is relayed. A turn that
//!   came from the chat is already there, and so is the reply its own ingress
//!   sent, so neither is sent twice; the bot's own posts are never delivered
//!   back to it as messages.
//! - **Only what the room may read.** The transcript withholds a member's
//!   entries from the others until they share with the group, and a chat is
//!   read by everyone in it, so a post or reply crosses only when every other
//!   member could read it in the app ([`readable_by_the_room`]). Otherwise it
//!   stays in the app, exactly as before.
//! - **Only a chat connected from the web app.** A group the chat created
//!   keeps its old behaviour; nothing is posted into it unasked.
//!
//! Best-effort: the post runs in the background once the row is stored, so a
//! chat that refuses it never fails or delays the turn.

use std::sync::Arc;

use pierre_core::models::groups::{CoachingGroup, GroupMember};
use pierre_core::models::{TenantId, TurnOrigin};
use pierre_database::database::ConversationRecord;
use pierre_services::group_chat_poster::{proactive_chat, GroupChatPoster};
use tokio::runtime::Handle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::context::ChatPipelineContext;
use crate::supervised::{spawn_supervised, TaskOwner};
use crate::surface_profile::SurfaceId;

/// Which row of a turn is relayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelayedRow {
    /// The member's own post.
    MemberPost,
    /// The agent's reply to it, attributed to the member in the transcript.
    AgentReply,
}

/// Whether a row `author` wrote (or received, for a reply) would be read by
/// every other member of `group` in the app — the transcript's own rule, for
/// an audience of all of them at once.
///
/// - The group's coach: their own posts are read by everyone; the agent's
///   reply to them may draw on any member's training, so it is read only
///   while every other live member shares with the group.
/// - A member: everything of theirs, post and reply alike, is read only while
///   the group shares and they consented to share with it.
/// - Anyone else reads as no one's.
#[must_use]
pub(crate) fn readable_by_the_room(
    group: &CoachingGroup,
    members: &[GroupMember],
    author: Uuid,
    row: RelayedRow,
) -> bool {
    let shares = |member: &GroupMember| {
        member.left_at.is_none() && group.peer_data_sharing && member.peer_sharing_consent
    };
    if group.coach_user_id == Some(author) {
        return match row {
            RelayedRow::MemberPost => true,
            RelayedRow::AgentReply => members
                .iter()
                .filter(|m| m.user_id != author && m.left_at.is_none())
                .all(shares),
        };
    }
    members
        .iter()
        .find(|m| m.user_id == author)
        .is_some_and(shares)
}

/// The name a relayed post is attributed with: the member's name in the
/// group, else their display name, else the part of their email before the
/// `@`. Never the whole address: the chat may hold people who are not in the
/// group.
fn relay_name(member: Option<&GroupMember>, display_name: Option<&str>, email: &str) -> String {
    member
        .and_then(|m| m.display_name.as_deref())
        .or(display_name)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map_or_else(
            || email.split('@').next().unwrap_or_default().to_owned(),
            str::to_owned,
        )
}

/// A group whose connected chat a row may be relayed into, with what the
/// relay needs to decide and to send.
struct RelayTarget {
    group: CoachingGroup,
    members: Vec<GroupMember>,
    poster: Arc<dyn GroupChatPoster>,
}

/// The group a stored row of a turn on `surface` would be relayed for: one
/// connected to a chat from the web app, on a turn from Dravr's own apps.
/// `None` when nothing is relayed; a failed lookup is logged and relays
/// nothing.
async fn relay_target(
    ctx: &ChatPipelineContext,
    surface: SurfaceId,
    conversation: &ConversationRecord,
) -> Option<RelayTarget> {
    if !matches!(surface, SurfaceId::Web | SurfaceId::Mobile) {
        return None;
    }
    let group_id = conversation.group_id.as_deref()?;
    let poster = ctx.tool_runtime.group_chat_poster().map(Arc::clone)?;
    // The id alone finds a group; the tenant argument filters nothing.
    let tenant = TenantId::parse_str(&conversation.tenant_id).ok()?;
    let groups = ctx.repos.groups.as_ref();
    let group = groups
        .get_group(group_id, tenant)
        .await
        .inspect_err(|e| warn!(error = %e, group_id, "room relay: group lookup failed"))
        .ok()
        .flatten()
        .filter(|group| group.is_active && group.chat_connection.is_some())?;
    let members = groups
        .list_members(group_id)
        .await
        .inspect_err(|e| warn!(error = %e, group_id, "room relay: member lookup failed"))
        .ok()?;
    Some(RelayTarget {
        group,
        members,
        poster,
    })
}

/// What the chat reads for `row`: the reply as it was written, a member's
/// post under their name.
async fn relay_body(
    ctx: &ChatPipelineContext,
    members: &[GroupMember],
    author: Uuid,
    row: RelayedRow,
    text: &str,
) -> String {
    if row == RelayedRow::AgentReply {
        return text.to_owned();
    }
    let user = ctx.repos.users.get_global(author).await.ok().flatten();
    let name = relay_name(
        members.iter().find(|m| m.user_id == author),
        user.as_ref().and_then(|u| u.display_name.as_deref()),
        user.as_ref().map_or("", |u| u.email.as_str()),
    );
    format!("{name}: {text}")
}

/// Relay one stored row of a web or mobile turn into its group's connected
/// chat, when it may cross (see the module docs).
pub(crate) async fn relay_to_connected_chat(
    ctx: &ChatPipelineContext,
    surface: SurfaceId,
    conversation: &ConversationRecord,
    author: &str,
    row: RelayedRow,
    text: &str,
) {
    let Ok(author) = Uuid::parse_str(author) else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }
    let Some(target) = relay_target(ctx, surface, conversation).await else {
        return;
    };
    let group_id = target.group.id;
    if !readable_by_the_room(&target.group, &target.members, author, row) {
        info!(%group_id, row = ?row, "room relay: the room may not read this row; kept in the app");
        return;
    }
    let Some(chat) = proactive_chat(&target.group) else {
        return;
    };
    let body = relay_body(ctx, &target.members, author, row, text).await;
    let (channel, chat_id, tenant_id) = (chat.channel, chat.chat_id.to_owned(), chat.tenant_id);
    let poster = target.poster;
    let owner = TaskOwner::Conversation(conversation.id.clone());
    spawn_supervised(&Handle::current(), "room_relay_post", owner, async move {
        let parts = poster.post(tenant_id, channel, &chat_id, &body).await;
        if parts == 0 {
            warn!(%group_id, channel = %channel, row = ?row, "room relay: nothing reached the chat");
        } else {
            info!(%group_id, channel = %channel, row = ?row, parts, "room relay: posted into the chat");
        }
    });
}

/// Relay the agent's reply to an athlete's own app turn: a turn the platform
/// started answers nobody's post, so its reply stays in the app.
pub(crate) async fn relay_reply(
    ctx: &ChatPipelineContext,
    surface: SurfaceId,
    origin: TurnOrigin,
    conversation: &ConversationRecord,
    author: &str,
    text: &str,
) {
    if origin == TurnOrigin::Athlete {
        relay_to_connected_chat(
            ctx,
            surface,
            conversation,
            author,
            RelayedRow::AgentReply,
            text,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::groups::{
        CoachConsents, GroupDigestMode, GroupRespondMode, GroupRole,
    };

    use super::*;

    fn group(coach: Option<Uuid>, peer_data_sharing: bool) -> CoachingGroup {
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: "Les Rouleurs".to_owned(),
            description: None,
            agent_id: "agent".to_owned(),
            owner_id: Uuid::new_v4(),
            coach_user_id: coach,
            peer_data_sharing,
            respond_mode: GroupRespondMode::All,
            digest_mode: GroupDigestMode::Off,
            max_members: 20,
            is_active: true,
            channel_type: Some("telegram".to_owned()),
            channel_chat_id: Some("-100".to_owned()),
            chat_connection: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn member(user_id: Uuid, shares: bool) -> GroupMember {
        GroupMember {
            id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            user_id,
            tenant_id: String::new(),
            role: GroupRole::Member,
            peer_sharing_consent: shares,
            coach_sharing_consent: true,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: Utc::now(),
            joined_at: Utc::now(),
            left_at: None,
            display_name: None,
        }
    }

    #[test]
    fn a_member_who_shares_with_the_group_is_relayed_post_and_reply() {
        let alice = Uuid::new_v4();
        let members = [member(alice, true), member(Uuid::new_v4(), false)];
        let g = group(None, true);
        assert!(
            readable_by_the_room(&g, &members, alice, RelayedRow::MemberPost),
            "bug: a sharing member's post is read by the whole room"
        );
        assert!(
            readable_by_the_room(&g, &members, alice, RelayedRow::AgentReply),
            "bug: the reply to a sharing member is read by the whole room"
        );
    }

    #[test]
    fn a_member_who_does_not_share_stays_in_the_app() {
        let alice = Uuid::new_v4();
        let members = [member(alice, false)];
        assert!(
            !readable_by_the_room(&group(None, true), &members, alice, RelayedRow::MemberPost),
            "bug: a member who does not share is withheld from the others"
        );
        let sharing = [member(alice, true)];
        assert!(
            !readable_by_the_room(&group(None, false), &sharing, alice, RelayedRow::MemberPost),
            "bug: the group's kill switch withholds everyone"
        );
    }

    #[test]
    fn the_coach_is_relayed_but_their_reply_only_while_every_member_shares() {
        let coach = Uuid::new_v4();
        let g = group(Some(coach), true);
        let all_share = [member(Uuid::new_v4(), true), member(Uuid::new_v4(), true)];
        let one_withholds = [member(Uuid::new_v4(), true), member(Uuid::new_v4(), false)];
        assert!(
            readable_by_the_room(&g, &one_withholds, coach, RelayedRow::MemberPost),
            "bug: the coach's own post is read by everyone"
        );
        assert!(
            readable_by_the_room(&g, &all_share, coach, RelayedRow::AgentReply),
            "bug: the reply to the coach crosses while every member shares"
        );
        assert!(
            !readable_by_the_room(&g, &one_withholds, coach, RelayedRow::AgentReply),
            "bug: the reply to the coach may draw on a withholding member, so it stays in the app"
        );
    }

    #[test]
    fn someone_outside_the_group_is_never_relayed() {
        let members = [member(Uuid::new_v4(), true)];
        assert!(
            !readable_by_the_room(
                &group(None, true),
                &members,
                Uuid::new_v4(),
                RelayedRow::MemberPost
            ),
            "bug: a row by someone outside the group reads as no one's"
        );
    }

    #[test]
    fn a_relayed_post_never_carries_a_whole_email_address() {
        assert_eq!(
            relay_name(None, None, "alice.martin@example.com"),
            "alice.martin",
            "bug: with no name, only the part before the `@` is shown"
        );
        assert_eq!(
            relay_name(None, Some("Alice"), "a@example.com"),
            "Alice",
            "bug: the display name wins over the email"
        );
        let mut named = member(Uuid::new_v4(), true);
        named.display_name = Some("Ali".to_owned());
        assert_eq!(
            relay_name(Some(&named), Some("Alice"), "a@example.com"),
            "Ali",
            "bug: the member's name in the group wins over their display name"
        );
        assert_eq!(
            relay_name(None, Some("  "), "bob@example.com"),
            "bob",
            "bug: a blank display name counts as none, so the email's local part is shown"
        );
    }
}
