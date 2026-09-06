// ABOUTME: Integration tests for GET /api/commands — the per-caller slash-command catalogue
// ABOUTME: Asserts the listing narrows and widens with the caller's real group standing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::coaches::{CoachCategory, CreateCoachRequest};
use pierre_core::models::groups::{CoachingGroup, GroupMember, GroupRespondMode, GroupRole};
use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse};
use pierre_mcp_server::routes::commands::{CommandCatalogueResponse, CommandEntry, CommandRoutes};
use serde_json::json;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// Reads `list_groups_for_user().first()`, so any membership at all lists it.
const ANY_GROUP_COMMAND: &str = "/group status";
/// Acts on the conversation's group and needs manage-members there, so a plain
/// member never sees it and an owner only sees it in that conversation.
const MANAGE_MEMBERS_COMMAND: &str = "/group invite";
/// No group precondition — every authenticated caller may run it.
const UNGATED_COMMAND: &str = "/plan";

async fn seed_user_tenant(resources: &Arc<ServerContext>, email: &str) -> (Uuid, TenantId, String) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Catalogue Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: "Catalogue Tenant".to_owned(),
        slug: format!("catalogue-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();

    (user_id, tenant_id, format!("Bearer {token}"))
}

async fn seed_coach(resources: &Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) -> String {
    let request = CreateCoachRequest {
        title: "Catalogue Coach".to_owned(),
        description: Some("Coach for the catalogue tests".to_owned()),
        system_prompt: "You are a test coach.".to_owned(),
        category: CoachCategory::Training,
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
    };
    let coach = resources
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    coach.id.to_string()
}

/// Put `user_id` in a coaching group with `role` and return the group id.
async fn seed_group_membership(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    coach_id: &str,
    role: GroupRole,
) -> Uuid {
    let now = chrono::Utc::now();
    let group_id = Uuid::new_v4();
    let group = CoachingGroup {
        id: group_id,
        tenant_id: tenant_id.to_string(),
        name: "Catalogue Group".to_owned(),
        description: None,
        coach_id: coach_id.to_owned(),
        owner_id: user_id,
        coach_user_id: None,
        peer_data_sharing: false,
        respond_mode: GroupRespondMode::default(),
        max_members: 20,
        is_active: true,
        channel_type: None,
        channel_chat_id: None,
        created_at: now,
        updated_at: now,
    };
    resources
        .common
        .repos
        .groups
        .create_group(tenant_id, &group)
        .await
        .unwrap();
    resources
        .common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id,
            user_id,
            tenant_id: tenant_id.to_string(),
            role,
            peer_sharing_consent: false,
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();

    group_id
}

/// Fetch the catalogue, optionally scoped to a conversation.
async fn fetch_catalogue(
    router: axum::Router,
    auth: &str,
    conversation_id: Option<&str>,
) -> Vec<CommandEntry> {
    let uri = conversation_id.map_or_else(
        || "/api/commands".to_owned(),
        |id| format!("/api/commands?conversation_id={id}"),
    );
    let resp = AxumTestRequest::get(&uri)
        .header("authorization", auth)
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let body: CommandCatalogueResponse = resp.json();
    body.commands
}

fn commands_of(entries: &[CommandEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.command.as_str()).collect()
}

async fn create_conversation(router: axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({"title": "Catalogue", "model": "gemini-1.5-flash"}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = resp.json();
    conv.id
}

/// Turns red if the catalogue stops carrying the frontmatter fields verbatim —
/// a client that has to reconstruct an argument signature or a description is
/// back to hardcoding a second list.
#[tokio::test]
async fn catalogue_entries_carry_the_frontmatter_verbatim() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-fields@test.com").await;
    let router = CommandRoutes::routes(Arc::clone(&resources));

    let entries = fetch_catalogue(router, &auth, None).await;

    let plan = entries
        .iter()
        .find(|e| e.command == UNGATED_COMMAND)
        .expect("/plan is ungated and must always be listed");
    assert_eq!(plan.name, "plan");
    assert_eq!(plan.domain, "training");
    assert_eq!(plan.args.as_deref(), Some("[week|today]"));
    // The description is read in the caller's locale (fr, the default) from
    // the five-locale registry, so it is the French line, not the English
    // frontmatter the catalogue file carries.
    let registry = &resources.mcp.messaging_strings_registry;
    assert_eq!(
        plan.description,
        registry.get("commands.plan.description", "fr"),
        "the palette description must be the registry's French line"
    );
    assert_ne!(
        plan.description,
        "Show your training plan — goal countdown plus today and tomorrow, the full week, or today alone",
        "a French reader must not get the English frontmatter"
    );

    // `/plan share` is deliberately absent here: a catalogue fetched without
    // a conversation answers for a solo thread, and the share variant is
    // listed only where a room exists to share into — see
    // `plan_share_is_listed_only_where_a_room_exists`, which pins its
    // frontmatter in the group-bound fetch.
    assert!(
        !entries.iter().any(|e| e.command == "/plan share"),
        "a solo catalogue must not offer the share variant beside /plan"
    );

    // A command with no `arguments:` in its frontmatter carries no signature,
    // rather than an empty string a client would render as trailing space.
    let help = entries
        .iter()
        .find(|e| e.command == "/help")
        .expect("/help has no precondition and must always be listed");
    assert_eq!(help.args, None);

    // The palette shows the argument hint the standard calls for: the athlete
    // learns `/agent add @handle` from the entry itself.
    let coach_add = entries
        .iter()
        .find(|e| e.command == "/agent add")
        .expect("/agent add has no precondition and must always be listed");
    assert_eq!(coach_add.name, "coach-add");
    assert_eq!(coach_add.domain, "coach");
    assert_eq!(coach_add.args.as_deref(), Some("@handle"));
    let coach_list = entries
        .iter()
        .find(|e| e.command == "/agent")
        .expect("/agent is the list and must always be listed");
    assert_eq!(coach_list.name, "coach-list");
}

/// Turns red if the catalogue answers for the caller's memberships instead
/// of the conversation: an owner typing alone must not be offered the
/// commands that act on "the group", because a solo thread names none and the
/// messaging DM's first-group fallback is off in the app.
#[tokio::test]
async fn owner_in_a_solo_thread_is_not_offered_group_management() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-solo-owner@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id).await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;

    let router = ChatRoutes::routes(Arc::clone(&resources))
        .merge(CommandRoutes::routes(Arc::clone(&resources)));
    let conversation_id = create_conversation(router.clone(), &auth).await;

    let entries = fetch_catalogue(router, &auth, Some(&conversation_id)).await;
    let listed = commands_of(&entries);

    for hidden in [
        "/group invite",
        "/group coach",
        "/group respond",
        "/group consent",
        "/coach invite",
    ] {
        assert!(
            !listed.contains(&hidden),
            "{hidden} acts on the thread's group, and a solo thread has none, got {listed:?}"
        );
    }
    assert!(
        listed.contains(&ANY_GROUP_COMMAND),
        "an owner belongs to a group, so {ANY_GROUP_COMMAND} stays listed, got {listed:?}"
    );
}

/// Turns red if a bound conversation stops answering per role: a plain member
/// in the group thread keeps the membership commands and is still not offered
/// the manage ones, exactly as a group room answers on messaging.
#[tokio::test]
async fn member_in_a_bound_conversation_is_listed_per_role() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-bound-member@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id).await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    let router = ChatRoutes::routes(Arc::clone(&resources))
        .merge(CommandRoutes::routes(Arc::clone(&resources)));
    let conversation_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conversation_id, Some(&group_id.to_string()), tenant_id)
        .await
        .unwrap();

    let entries = fetch_catalogue(router, &auth, Some(&conversation_id)).await;
    let listed = commands_of(&entries);

    assert!(
        listed.contains(&"/group consent"),
        "a member of the thread's group may set their consent there, got {listed:?}"
    );
    assert!(
        listed.contains(&ANY_GROUP_COMMAND),
        "a member belongs to a group, so {ANY_GROUP_COMMAND} must be listed, got {listed:?}"
    );
    for hidden in [MANAGE_MEMBERS_COMMAND, "/group coach", "/coach invite"] {
        assert!(
            !listed.contains(&hidden),
            "{hidden} needs manage standing in the thread's group, got {listed:?}"
        );
    }
}

/// Turns red if the catalogue stops filtering on group standing — the palette
/// would then offer an athlete in no group commands that refuse them.
#[tokio::test]
async fn athlete_in_no_group_gets_no_group_commands() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-solo@test.com").await;
    let router = CommandRoutes::routes(Arc::clone(&resources));

    let entries = fetch_catalogue(router, &auth, None).await;
    let listed = commands_of(&entries);

    assert!(
        listed.contains(&UNGATED_COMMAND),
        "an ungated command must still be listed, got {listed:?}"
    );
    assert!(
        !listed.contains(&ANY_GROUP_COMMAND),
        "{ANY_GROUP_COMMAND} needs a membership this caller does not have, got {listed:?}"
    );
    assert!(
        !listed.contains(&MANAGE_MEMBERS_COMMAND),
        "{MANAGE_MEMBERS_COMMAND} needs manage-members standing, got {listed:?}"
    );
}

/// Turns red if the two group predicates collapse into one — a plain member
/// would then be offered the invite command that refuses them, which is the
/// exact lie the runtime resolution exists to prevent.
#[tokio::test]
async fn plain_member_sees_the_membership_command_but_not_the_manage_one() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-member@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id).await;
    seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;
    let router = CommandRoutes::routes(Arc::clone(&resources));

    let entries = fetch_catalogue(router, &auth, None).await;
    let listed = commands_of(&entries);

    assert!(
        listed.contains(&ANY_GROUP_COMMAND),
        "a member belongs to a group, so {ANY_GROUP_COMMAND} must be listed, got {listed:?}"
    );
    assert!(
        !listed.contains(&MANAGE_MEMBERS_COMMAND),
        "a plain member cannot manage members, so {MANAGE_MEMBERS_COMMAND} must not be listed, got {listed:?}"
    );
}

/// Turns red if the catalogue stops reading `conversation_id` — the owner's
/// conversation-scoped commands would vanish from the palette that is open in
/// that very conversation.
#[tokio::test]
async fn owner_sees_the_manage_command_in_the_bound_conversation() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) = seed_user_tenant(&resources, "catalogue-owner@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id).await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Owner).await;

    let router = ChatRoutes::routes(Arc::clone(&resources))
        .merge(CommandRoutes::routes(Arc::clone(&resources)));
    let conversation_id = create_conversation(router.clone(), &auth).await;
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conversation_id, Some(&group_id.to_string()), tenant_id)
        .await
        .unwrap();

    let entries = fetch_catalogue(router, &auth, Some(&conversation_id)).await;
    let listed = commands_of(&entries);

    assert!(
        listed.contains(&MANAGE_MEMBERS_COMMAND),
        "an owner of the conversation's group may invite, got {listed:?}"
    );
    assert!(
        listed.contains(&ANY_GROUP_COMMAND),
        "an owner is also a member, got {listed:?}"
    );
}

/// Turns red if the catalogue ever answers unauthenticated. It resolves the
/// caller's own standing, so serving it without an identity would be serving
/// somebody else's answer.
#[tokio::test]
async fn catalogue_requires_authentication() {
    let resources = create_test_server_resources().await.unwrap();
    let router = CommandRoutes::routes(Arc::clone(&resources));

    let resp = AxumTestRequest::get("/api/commands").send(router).await;

    assert_eq!(resp.status_code(), StatusCode::UNAUTHORIZED);
}

/// Turns red if the ordering becomes the registry's `HashMap` iteration order —
/// the palette would then reshuffle itself on every server restart.
#[tokio::test]
async fn catalogue_is_ordered_by_domain_then_command() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-order@test.com").await;
    let router = CommandRoutes::routes(Arc::clone(&resources));

    let entries = fetch_catalogue(router, &auth, None).await;

    let keys: Vec<(String, String)> = entries
        .iter()
        .map(|e| (e.domain.clone(), e.command.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(
        keys, sorted,
        "catalogue must be sorted by domain then command"
    );
    assert!(
        entries.len() >= 2,
        "the catalogue must carry real commands, got {}",
        entries.len()
    );
}

/// Turns red if the share variant leaks into a solo palette or vanishes from
/// a room one. `/plan share` renders identically to `/plan` in a DM, so a
/// palette with no conversation (and a solo thread) must not list it, while a
/// group-bound conversation — the only thread with a room to share into —
/// must, with its frontmatter verbatim.
#[tokio::test]
async fn plan_share_is_listed_only_where_a_room_exists() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "catalogue-plan-share@test.com").await;
    let coach_id = seed_coach(&resources, user_id, tenant_id).await;
    let group_id =
        seed_group_membership(&resources, user_id, tenant_id, &coach_id, GroupRole::Member).await;

    let router = ChatRoutes::routes(Arc::clone(&resources))
        .merge(CommandRoutes::routes(Arc::clone(&resources)));

    // No conversation: a palette opened outside any thread is personal.
    let solo = fetch_catalogue(router.clone(), &auth, None).await;
    assert!(
        solo.iter().any(|e| e.command == UNGATED_COMMAND),
        "/plan stays listed in a solo palette, got {:?}",
        commands_of(&solo)
    );
    assert!(
        !solo.iter().any(|e| e.command == "/plan share"),
        "in a solo palette the share variant duplicates /plan, got {:?}",
        commands_of(&solo)
    );

    // A solo conversation binds no group: still no share variant.
    let conversation_id = create_conversation(router.clone(), &auth).await;
    let solo_thread = fetch_catalogue(router.clone(), &auth, Some(&conversation_id)).await;
    assert!(
        !solo_thread.iter().any(|e| e.command == "/plan share"),
        "a solo thread has no room to share into, got {:?}",
        commands_of(&solo_thread)
    );

    // The group-bound conversation is a room: listed, frontmatter verbatim.
    resources
        .common
        .repos
        .chat
        .set_conversation_group_id(&conversation_id, Some(&group_id.to_string()), tenant_id)
        .await
        .unwrap();
    let bound = fetch_catalogue(router, &auth, Some(&conversation_id)).await;
    let share = bound
        .iter()
        .find(|e| e.command == "/plan share")
        .unwrap_or_else(|| {
            panic!(
                "/plan share must be listed in a group-bound conversation, got {:?}",
                commands_of(&bound)
            )
        });
    assert_eq!(share.name, "plan-share");
    assert_eq!(share.domain, "training");
    assert_eq!(share.args.as_deref(), Some("[week|today]"));
    assert!(
        share.description.contains("dans la salle"),
        "the description says where the reply goes, in the reader's language: {}",
        share.description
    );
}
