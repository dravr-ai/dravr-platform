// ABOUTME: A coach's correction posted into the group's bound Telegram, Slack or Discord chat, when the agent sent the reply there
// ABOUTME: Named, quoted and localised through humanCoach.reply_corrected; best-effort, the correction is already in the group thread
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_contremaitre::messaging_strings::{MessagingStringsRegistry, KEY_GROUP_REPLY_CORRECTED};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{CoachReply, CoachingGroup, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_routes_groups::GroupChatPoster;
use pierre_services::coach_replies::CorrectionOutcome;
use pierre_services::locale::resolve_user_locale;
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::routes::chat::preview_text;
use crate::services::group_chat_poster::ServerGroupChatPoster;

/// Post `outcome`'s correction into its group's bound chat; `true` when
/// the chat accepted it. A group bound to no chat, or one the read of
/// which fails, is reported as not posted and logged.
///
/// Only a reply the agent sent in that chat is corrected there. A reply
/// written in the athlete's app thread never reached the chat, and its
/// readers are the room's consent rule's alone: quoting it and naming the
/// athlete to every member of the chat would show it to peers the athlete
/// does not share with. Its correction stays in the group thread, where the
/// athlete read the reply.
pub(super) async fn post(
    resources: &ServerContext,
    tenant_id: TenantId,
    coach_user_id: Uuid,
    outcome: &CorrectionOutcome,
) -> bool {
    let repos = &resources.common.repos;
    let group_id = outcome.reply.group_id;
    let group = match repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
    {
        Ok(Some(group)) => group,
        Ok(None) => return false,
        Err(e) => {
            warn!(group_id = %group_id, error = %e, "correction: group unreadable; not posted to its chat");
            return false;
        }
    };
    if !sent_in_group_chat(repos, &outcome.reply, &group).await {
        return false;
    }
    // The chat reads who corrected whose reply; a post that cannot name both
    // is not made, and the correction stays in the group thread.
    let Some(coach_name) = chat_name(repos, coach_user_id, group_id).await else {
        return false;
    };
    let Some(athlete_name) = chat_name(repos, outcome.reply.athlete_user_id, group_id).await else {
        return false;
    };
    let locale = resolve_user_locale(repos.users.as_ref(), outcome.reply.athlete_user_id).await;
    let poster = ServerGroupChatPoster::from_repos(Arc::clone(repos));
    post_into_group_chat(
        poster.as_ref(),
        &resources.mcp.messaging_strings_registry,
        &group,
        &locale,
        ChatNames {
            coach: &coach_name,
            athlete: &athlete_name,
        },
        outcome,
    )
    .await
}

/// Who a correction names in the group's chat: the coach who wrote it and
/// the athlete whose reply it corrects.
#[derive(Clone, Copy)]
struct ChatNames<'a> {
    coach: &'a str,
    athlete: &'a str,
}

/// A person's name as the group's chat reads it: their display name, else
/// the local part of their email — never the whole address, which every
/// member of the chat would read (the rule `athlete_display_name` applies to
/// a room). `None`, logged, when the account is gone or unreadable.
async fn chat_name(repos: &RepositoryRegistry, user_id: Uuid, group_id: Uuid) -> Option<String> {
    match repos.users.get_global(user_id).await {
        Ok(Some(user)) => Some(name_for_chat(user.display_name, &user.email)),
        Ok(None) => {
            warn!(group_id = %group_id, user_id = %user_id, "correction: a named account is gone; not posted to the group's chat");
            None
        }
        Err(e) => {
            warn!(group_id = %group_id, user_id = %user_id, error = %e, "correction: a name is unreadable; not posted to the group's chat");
            None
        }
    }
}

/// A display name that is set, else the local part of `email`.
fn name_for_chat(display_name: Option<String>, email: &str) -> String {
    display_name
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_owned())
}

/// Whether `reply` was delivered into `group`'s bound chat: the athlete's
/// conversation it was written in belongs to the messaging session of that
/// chat. A reply from no conversation, from an app thread or from another
/// chat was not, and neither is one whose session cannot be read (logged).
async fn sent_in_group_chat(
    repos: &RepositoryRegistry,
    reply: &CoachReply,
    group: &CoachingGroup,
) -> bool {
    let Some(conversation_id) = reply.conversation_id.as_deref() else {
        return false;
    };
    let Ok(tenant) = TenantId::parse_str(&reply.conversation_tenant_id) else {
        warn!(group_id = %group.id, "correction: the reply's conversation tenant is not a uuid; not posted to the group's chat");
        return false;
    };
    match repos
        .messaging
        .get_session_by_pierre_conversation_id(tenant, conversation_id)
        .await
    {
        Ok(Some(session)) => session_is_group_chat(&session, group),
        Ok(None) => false,
        Err(e) => {
            warn!(group_id = %group.id, error = %e, "correction: the reply's messaging session is unreadable; not posted to the group's chat");
            false
        }
    }
}

/// Whether a messaging `session` is the chat `group` is bound to: the same
/// channel and the same channel-native chat, the pair the ingress resolves a
/// group from.
fn session_is_group_chat(session: &Value, group: &CoachingGroup) -> bool {
    let (Some(channel), Some(chat_id)) = (
        group.channel_type.as_deref(),
        group.channel_chat_id.as_deref(),
    ) else {
        return false;
    };
    session.get("channel_type").and_then(Value::as_str) == Some(channel)
        && session
            .get("channel_conversation_id")
            .and_then(Value::as_str)
            == Some(chat_id)
}

/// The text of a correction as a chat reads it: who corrected the reply
/// to whom, the reply's opening, then the correction.
fn correction_text(
    registry: &MessagingStringsRegistry,
    locale: &str,
    names: ChatNames<'_>,
    outcome: &CorrectionOutcome,
) -> String {
    let excerpt = preview_text(&outcome.reply.content);
    registry.render(
        KEY_GROUP_REPLY_CORRECTED,
        locale,
        &[
            names.coach,
            names.athlete,
            &excerpt,
            &outcome.correction.content,
        ],
    )
}

/// Post the correction into `group`'s bound chat through `poster`; `true`
/// when at least one part reached it. A group bound to no chat is not
/// posted to, and neither is one bound to `WhatsApp` or Messenger, which
/// refuse a text the platform starts outside the 24-hour window after a
/// member's last message — the weekly digest's rule for the same chats.
async fn post_into_group_chat(
    poster: &dyn GroupChatPoster,
    registry: &MessagingStringsRegistry,
    group: &CoachingGroup,
    locale: &str,
    names: ChatNames<'_>,
    outcome: &CorrectionOutcome,
) -> bool {
    let (Some(channel), Some(chat_id)) = (
        group.channel_type.as_deref(),
        group.channel_chat_id.as_deref(),
    ) else {
        return false;
    };
    let Some(channel_type) = channel.parse::<ChannelType>().ok().filter(|c| {
        matches!(
            c,
            ChannelType::Telegram | ChannelType::Slack | ChannelType::Discord
        )
    }) else {
        info!(group_id = %group.id, channel = %channel, "correction: the group's chat takes no post the platform starts; it is in the group thread only");
        return false;
    };
    let Ok(tenant) = TenantId::parse_str(&group.tenant_id) else {
        warn!(group_id = %group.id, "correction: the group's tenant is not a uuid; not posted to its chat");
        return false;
    };
    let text = correction_text(registry, locale, names, outcome);
    let parts = poster.post(tenant, channel_type, chat_id, &text).await;
    if parts > 0 {
        info!(group_id = %group.id, channel = %channel, parts, "correction: posted into the group's chat");
    }
    parts > 0
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use pierre_core::models::groups::{GroupDigestMode, GroupRespondMode};
    use pierre_core::models::CoachReplyCorrection;
    use pierre_core::transport::TransportPolicy;
    use serde_json::json;

    use super::*;

    /// A poster that records what it was handed and accepts it as one part.
    #[derive(Default)]
    struct RecordingPoster {
        posts: Mutex<Vec<(TenantId, ChannelType, String, String)>>,
    }

    #[async_trait]
    impl GroupChatPoster for RecordingPoster {
        async fn post(
            &self,
            tenant_id: TenantId,
            channel_type: ChannelType,
            chat_id: &str,
            text: &str,
        ) -> usize {
            self.posts.lock().unwrap().push((
                tenant_id,
                channel_type,
                chat_id.to_owned(),
                text.to_owned(),
            ));
            1
        }
    }

    fn group(channel: Option<(&str, &str)>) -> CoachingGroup {
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: "Marathon block".to_owned(),
            description: None,
            agent_id: "agent".to_owned(),
            owner_id: Uuid::new_v4(),
            coach_user_id: None,
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::All,
            digest_mode: GroupDigestMode::Off,
            max_members: 20,
            is_active: true,
            channel_type: channel.map(|(c, _)| c.to_owned()),
            channel_chat_id: channel.map(|(_, id)| id.to_owned()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn outcome() -> CorrectionOutcome {
        let entry_id = Uuid::new_v4();
        CorrectionOutcome {
            reply: CoachReply {
                entry_id,
                group_id: Uuid::new_v4(),
                group_name: "Marathon block".to_owned(),
                conversation_tenant_id: Uuid::new_v4().to_string(),
                athlete_user_id: Uuid::new_v4(),
                athlete_name: Some("Paul".to_owned()),
                content: "Your **2:41** marathon in May was a big step.".to_owned(),
                conversation_id: Some("c1".to_owned()),
                message_id: Some("m1".to_owned()),
                created_at: Utc::now(),
                transport_policy: TransportPolicy::AnyTransport,
            },
            correction: CoachReplyCorrection {
                id: Uuid::new_v4(),
                reply_entry_id: entry_id,
                correction_entry_id: Uuid::new_v4(),
                content: "Paul has not run a marathon yet; the May race was a half.".to_owned(),
                created_at: Utc::now(),
            },
        }
    }

    /// The coach and the athlete the chat names.
    const NAMES: ChatNames<'static> = ChatNames {
        coach: "Coach Marie",
        athlete: "Paul",
    };

    /// A registry holding the one key this module renders, so the test
    /// pins what reaches the chat rather than the catalogue's wording.
    fn registry() -> MessagingStringsRegistry {
        let registry = MessagingStringsRegistry::new();
        registry.update(
            KEY_GROUP_REPLY_CORRECTED,
            "en",
            "{0} corrected the reply to {1} ({2}):\n{3}".to_owned(),
            "sha".to_owned(),
        );
        registry
    }

    #[tokio::test]
    async fn a_chat_bound_group_gets_the_correction_with_who_and_which_reply() {
        let poster = RecordingPoster::default();
        let group = group(Some(("telegram", "-100123")));
        let reached_chat =
            post_into_group_chat(&poster, &registry(), &group, "en", NAMES, &outcome()).await;
        assert!(reached_chat);
        let posts = poster.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        let (tenant, channel, chat_id, text) = &posts[0];
        assert_eq!(tenant.to_string(), group.tenant_id);
        assert_eq!(*channel, ChannelType::Telegram);
        assert_eq!(chat_id, "-100123");
        assert_eq!(
            text,
            "Coach Marie corrected the reply to Paul (Your 2:41 marathon in May was a big step.):\n\
             Paul has not run a marathon yet; the May race was a half."
        );
    }

    #[tokio::test]
    async fn a_whatsapp_group_is_not_posted_to() {
        let poster = RecordingPoster::default();
        let group = group(Some(("whatsapp", "120363")));
        let reached_chat =
            post_into_group_chat(&poster, &registry(), &group, "en", NAMES, &outcome()).await;
        assert!(!reached_chat);
        assert!(poster.posts.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_group_bound_to_no_chat_is_not_posted_to() {
        let poster = RecordingPoster::default();
        let reached_chat =
            post_into_group_chat(&poster, &registry(), &group(None), "en", NAMES, &outcome()).await;
        assert!(!reached_chat);
        assert!(poster.posts.lock().unwrap().is_empty());
    }

    #[test]
    fn the_chat_reads_a_display_name_else_the_local_part_never_the_address() {
        assert_eq!(
            name_for_chat(Some("Paul R".to_owned()), "paul@example.com"),
            "Paul R"
        );
        assert_eq!(name_for_chat(None, "paul@example.com"), "paul");
        assert_eq!(
            name_for_chat(Some("   ".to_owned()), "paul@example.com"),
            "paul"
        );
    }

    /// A messaging session as the repository projects it.
    fn session(channel: &str, chat_id: Option<&str>) -> Value {
        json!({
            "channel_type": channel,
            "channel_user_id": "4242",
            "channel_conversation_id": chat_id,
            "pierre_conversation_id": "c1",
        })
    }

    #[test]
    fn only_the_session_of_the_groups_own_chat_is_that_chat() {
        let bound = group(Some(("telegram", "-100123")));
        // The reply was sent in the group's chat: its readers saw it there.
        assert!(session_is_group_chat(
            &session("telegram", Some("-100123")),
            &bound
        ));
        // The athlete's direct messages with the bot: not the room.
        assert!(!session_is_group_chat(&session("telegram", None), &bound));
        // Another Telegram group the athlete is in.
        assert!(!session_is_group_chat(
            &session("telegram", Some("-100999")),
            &bound
        ));
        // The same chat id on another channel is another chat.
        assert!(!session_is_group_chat(
            &session("slack", Some("-100123")),
            &bound
        ));
        // A group bound to no chat has no session of its own.
        assert!(!session_is_group_chat(
            &session("telegram", Some("-100123")),
            &group(None)
        ));
    }
}
