// ABOUTME: /group digest — who may change where a group's weekly digest goes, and what each caller reads back
// ABOUTME: Runs the handler against a real group: owner and attached coach change it, a plain member is refused
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use chrono::Utc;
use pierre_commands::group::{GroupDigestHandler, GroupStatusHandler};
use pierre_commands::{
    caller_group_standing, CommandHandler, ConversationRotation, PlatformCommandContext,
};
use pierre_core::models::agents::{AgentCategory, CreateAgentRequest};
use pierre_core::models::groups::{
    CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use uuid::Uuid;

use common::create_test_server_resources;

const EN_OFF: &str = "Weekly recap: off — nothing is sent. To turn it on: /group digest chat \
                      (posted in the group's chat) or /group digest managers (owner and admins only).";
const EN_MANAGERS: &str = "Weekly recap: sent to the owner and admins only, on their own \
                           channels; nothing is posted in the group's chat. To change it: \
                           /group digest chat or /group digest off.";
const EN_FORBIDDEN: &str =
    "Only the group's owner, its admins and its human coach can change the weekly recap.";
const EN_USAGE: &str = "Usage: /group digest off, /group digest chat or /group digest managers";
const EN_TIER_OFF: &str =
    "Your plan does not include the weekly recap, so nothing is sent until it does.";
const FR_CHAT: &str = "Récap hebdo : publié chaque lundi matin dans la discussion Telegram, \
                       Slack ou Discord du groupe, avec le récap complet pour le propriétaire et \
                       les admins dans l'app. Pour changer : /group digest managers ou /group \
                       digest off.";

/// A group with an owner, a plain member and an attached human coach, each
/// with a conversation bound to it the way a shared room's rows are.
struct Room {
    resources: Arc<ServerContext>,
    tenant: TenantId,
    group_id: Uuid,
    owner: Caller,
    member: Caller,
    coach: Caller,
}

/// One person in the room and the conversation their turns arrive on.
struct Caller {
    user_id: Uuid,
    conversation_id: String,
}

impl Room {
    /// The room on a tenant on `plan`, its digest off.
    async fn on_plan(plan: &str) -> Self {
        let resources = create_test_server_resources().await.unwrap();
        let owner_id = person(&resources, "owner").await;
        let tenant = TenantId::generate();
        resources
            .common
            .repos
            .tenants
            .create(&Tenant {
                id: tenant,
                name: "Digest command tenant".to_owned(),
                slug: format!("digest-cmd-{tenant}"),
                domain: None,
                plan: plan.to_owned(),
                owner_user_id: owner_id,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
        let member_id = person(&resources, "member").await;
        let coach_id = person(&resources, "coach").await;
        let group_id = seed_group(&resources, tenant, owner_id, member_id).await;
        assert!(resources
            .common
            .repos
            .groups
            .set_group_coach_user(&group_id.to_string(), Some(coach_id), tenant)
            .await
            .unwrap());

        let mut callers = Vec::new();
        for user_id in [owner_id, member_id, coach_id] {
            let conversation = resources
                .common
                .repos
                .chat
                .create_conversation(
                    &user_id.to_string(),
                    tenant,
                    "Room",
                    "test-model",
                    None,
                    Some(&group_id.to_string()),
                )
                .await
                .unwrap();
            callers.push(Caller {
                user_id,
                conversation_id: conversation.id,
            });
        }
        let coach = callers.pop().unwrap();
        let member = callers.pop().unwrap();
        let owner = callers.pop().unwrap();
        Self {
            resources,
            tenant,
            group_id,
            owner,
            member,
            coach,
        }
    }

    /// A turn from `caller` in the room, in `locale`, carrying `args`.
    fn ctx(&self, caller: &Caller, locale: &str, args: &[&str]) -> PlatformCommandContext {
        PlatformCommandContext {
            user_id: caller.user_id,
            tenant_id: self.tenant,
            channel_type: "telegram".to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            raw_text: format!("/group digest {}", args.join(" ")),
            ctx: Arc::<ServerContext>::clone(&self.resources),
            locale: locale.to_owned(),
            is_direct_message: false,
            ambient_group_fallback: false,
            conversation_id: Some(caller.conversation_id.clone()),
            conversation_tenant_id: self.tenant,
            sender_id: None,
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&self.resources),
        }
    }

    async fn run(&self, caller: &Caller, locale: &str, args: &[&str]) -> String {
        GroupDigestHandler
            .execute(&self.ctx(caller, locale, args))
            .await
            .unwrap()
            .text
    }

    async fn stored_mode(&self) -> GroupDigestMode {
        self.resources
            .common
            .repos
            .groups
            .get_group(&self.group_id.to_string(), self.tenant)
            .await
            .unwrap()
            .expect("the group exists")
            .digest_mode
    }
}

async fn person(resources: &ServerContext, role: &str) -> Uuid {
    let mut user = User::new(
        format!("digest-{role}-{}@example.com", Uuid::new_v4()),
        "not-a-login".to_owned(),
        Some(format!("Digest {role}")),
    );
    user.user_status = UserStatus::Active;
    resources.common.repos.users.create(&user).await.unwrap()
}

/// The group, its digest off, with the owner and a plain member enrolled.
async fn seed_group(
    resources: &ServerContext,
    tenant: TenantId,
    owner: Uuid,
    member: Uuid,
) -> Uuid {
    let repos = &resources.common.repos;
    let agent = repos
        .agents
        .create(
            owner,
            tenant,
            &CreateAgentRequest {
                title: "Digest Agent".to_owned(),
                description: None,
                system_prompt: "You are a test agent.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let now = Utc::now();
    repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id,
                tenant_id: tenant.to_string(),
                name: "Les Rouleurs".to_owned(),
                description: None,
                agent_id: agent.id.to_string(),
                owner_id: owner,
                coach_user_id: None,
                peer_data_sharing: true,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 20,
                is_active: true,
                channel_type: Some("telegram".to_owned()),
                channel_chat_id: Some("-100424242".to_owned()),
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();
    for (user_id, role) in [(owner, GroupRole::Owner), (member, GroupRole::Member)] {
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: id,
                user_id,
                tenant_id: tenant.to_string(),
                role,
                peer_sharing_consent: false,
                consent_given_at: now,
                joined_at: now,
                left_at: None,
                display_name: None,
            })
            .await
            .unwrap();
    }
    id
}

/// The owner turns the digest on from the room, and the reply — the status of
/// the mode the group is now in — reads in the owner's own language.
#[tokio::test]
async fn the_owner_turns_the_digest_on_and_reads_it_in_their_language() {
    let room = Room::on_plan("professional").await;
    assert_eq!(room.stored_mode().await, GroupDigestMode::Off);

    let reply = room.run(&room.owner, "fr", &["chat"]).await;

    assert_eq!(reply, FR_CHAT);
    assert_eq!(room.stored_mode().await, GroupDigestMode::Chat);
}

/// The human coach attached to the group holds no membership in it, and may
/// still change where the digest goes.
#[tokio::test]
async fn the_attached_coach_changes_the_digest_mode() {
    let room = Room::on_plan("professional").await;
    assert!(room
        .resources
        .common
        .repos
        .groups
        .get_member(&room.group_id.to_string(), room.coach.user_id)
        .await
        .unwrap()
        .is_none());

    let reply = room.run(&room.coach, "en", &["MANAGERS"]).await;

    assert_eq!(reply, EN_MANAGERS);
    assert_eq!(room.stored_mode().await, GroupDigestMode::Managers);
}

/// A plain member is refused, and the group keeps its mode.
#[tokio::test]
async fn a_plain_member_is_refused_and_nothing_changes() {
    let room = Room::on_plan("professional").await;

    let reply = room.run(&room.member, "en", &["chat"]).await;

    assert_eq!(reply, EN_FORBIDDEN);
    assert_eq!(room.stored_mode().await, GroupDigestMode::Off);
}

/// With no argument anyone in the room reads the current mode and how to
/// change it; an argument that is not a mode reads the usage and changes
/// nothing.
#[tokio::test]
async fn no_argument_reports_the_mode_and_a_wrong_one_the_usage() {
    let room = Room::on_plan("professional").await;

    assert_eq!(room.run(&room.member, "en", &[]).await, EN_OFF);
    assert_eq!(room.run(&room.owner, "en", &["weekly"]).await, EN_USAGE);
    assert_eq!(room.stored_mode().await, GroupDigestMode::Off);
}

/// On a plan without the weekly digest the mode is stored — the tier is a
/// ceiling over delivery, not over the setting — and the reply says that
/// nothing goes out.
#[tokio::test]
async fn a_plan_without_the_digest_stores_the_mode_and_says_nothing_is_sent() {
    let room = Room::on_plan("starter").await;

    let reply = room.run(&room.owner, "en", &["managers"]).await;

    assert_eq!(reply, format!("{EN_MANAGERS}\n{EN_TIER_OFF}"));
    assert_eq!(room.stored_mode().await, GroupDigestMode::Managers);
}

/// `/group status` carries the digest line and how to change it.
#[tokio::test]
async fn group_status_shows_the_digest_mode() {
    let room = Room::on_plan("professional").await;
    let mut ctx = room.ctx(&room.owner, "en", &[]);
    ctx.is_direct_message = true;
    ctx.ambient_group_fallback = true;
    ctx.conversation_id = None;

    let reply = GroupStatusHandler.execute(&ctx).await.unwrap().text;

    assert!(reply.starts_with("Les Rouleurs stats:"), "{reply}");
    assert!(reply.ends_with(&format!("\n{EN_OFF}")), "{reply}");
}

/// `/help` and the command palette list `/group digest` for exactly the
/// callers the handler would obey: the owner and the attached coach.
#[tokio::test]
async fn the_command_is_listed_for_the_owner_and_the_coach_only() {
    let room = Room::on_plan("professional").await;

    for (caller, listed) in [
        (&room.owner, true),
        (&room.coach, true),
        (&room.member, false),
    ] {
        let standing = caller_group_standing(&room.ctx(caller, "en", &[]))
            .await
            .unwrap();
        assert_eq!(
            GroupDigestHandler.is_available(&standing),
            listed,
            "user {}",
            caller.user_id
        );
    }
}
