// ABOUTME: Integration tests for /discover, /discover install, /group create and /group join over the web chat route
// ABOUTME: Content-asserting: real published coaches, real groups and invites, every reply checked against the database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use axum::http::StatusCode;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use helpers::coach_fixtures::{publish_catalogue_coach, publish_catalogue_coach_in};
use helpers::notify_capture::{capture_notify, named, only};
use pierre_config::admin_types::{ConfigDataType, ConfigScope};
use pierre_contremaitre::messaging_strings::{
    KEY_DISCOVER_ADD_LABEL, KEY_DISCOVER_CARD_TITLE, KEY_DISCOVER_CATALOGUE_EMPTY,
    KEY_DISCOVER_EMPTY, KEY_DISCOVER_INSTALLED, KEY_DISCOVER_INSTALL_ALREADY,
    KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, KEY_DISCOVER_INSTALL_USAGE, KEY_DISCOVER_MORE_LABEL,
    KEY_GROUP_CREATED, KEY_GROUP_CREATE_FORBIDDEN, KEY_GROUP_CREATE_NO_COACH,
    KEY_GROUP_CREATE_USAGE, KEY_GROUP_INVITE_LABEL, KEY_GROUP_JOINED, KEY_GROUP_JOINED_AS_COACH,
    KEY_GROUP_JOIN_ALREADY_MEMBER, KEY_GROUP_JOIN_INVALID_CODE,
};
use pierre_core::models::coaches::{CoachCategory, CoachHandle, CreateCoachRequest};
use pierre_core::models::groups::{CreateGroupRequest, GroupInviteKind, GroupRole};
use pierre_core::models::{
    default_locale, AddMessageParams, ConnectionType, Tenant, TenantId, User, UserStatus,
    COMMAND_FINISH_REASON,
};
use pierre_groups::creation_policy::GROUP_CREATION_POLICY_KEY;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_mcp_server::config::admin::repository::SetOverrideParams;
use pierre_mcp_server::config::admin::AdminConfigManager;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{
    ChatMessageAction, ChatRoutes, ConversationResponse, ReplyBlockResponse, TurnResponse,
};
use serde_json::json;
use std::sync::Arc;
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// The model every test conversation is opened on; `/group create` copies it
/// onto the group conversation it files.
const MODEL: &str = "gemini-1.5-flash";
/// Telegram refuses a callback payload above this many bytes, so every
/// postback a card carries must fit.
const TELEGRAM_CALLBACK_LIMIT: usize = 64;
/// Admin-config category the group-creation policy is filed under.
const GROUP_PERMISSIONS_CATEGORY: &str = "group_permissions";

// ============================================================================
// Harness
// ============================================================================

/// An active user owning a tenant on `plan`, with a real provider connection
/// so the onboarding gate lets the slash dispatcher run. Returns the bearer
/// header ready to send.
async fn seed_user_tenant(
    resources: &Arc<ServerContext>,
    email: &str,
    plan: &str,
) -> (Uuid, TenantId, String) {
    let user = active_user(email, false).await;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant of {email}"),
        slug: format!("discover-{tenant_id}"),
        domain: None,
        plan: plan.to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    // tenants.create() files the owner in tenant_users with the owner role.
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();
    let auth = finish_user(resources, &user, tenant_id).await;
    (user_id, tenant_id, auth)
}

/// A plain member of an existing tenant — no owner or admin role, so the
/// tenant's `group_creation_policy` decides whether they may create groups.
/// `manages_roster` marks a roster-managing coach account.
async fn seed_tenant_member(
    resources: &Arc<ServerContext>,
    email: &str,
    tenant_id: TenantId,
    manages_roster: bool,
) -> (Uuid, String) {
    let user = active_user(email, manages_roster).await;
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    // Membership rows are written by the invitation flows in production; the
    // repository exposes no direct writer, so the fixture files the row the
    // way those flows do.
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) \
         VALUES (?, ?, ?, 'member', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(tenant_id.to_string())
    .bind(user_id.to_string())
    .bind(&now)
    .bind(&now)
    .execute(resources.coach.database.sqlite_pool().unwrap())
    .await
    .unwrap();
    let auth = finish_user(resources, &user, tenant_id).await;
    (user_id, auth)
}

async fn active_user(email: &str, manages_roster: bool) -> User {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Discover Test".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    user.manages_roster = manages_roster;
    user
}

/// Point the user at `tenant_id`, give them a non-synthetic provider
/// connection and mint their bearer header.
async fn finish_user(resources: &Arc<ServerContext>, user: &User, tenant_id: TenantId) -> String {
    let repos = &resources.common.repos;
    repos
        .users
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();
    repos
        .provider_connections
        .register_connection(user.id, tenant_id, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    format!("Bearer {}", generate_test_token(resources, user).await)
}

/// A private coach owned by `user_id`, selected as their default so
/// `/group create` in a coach-less thread has a coach to build on.
async fn seed_selected_coach(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> String {
    let request: CreateCoachRequest = serde_json::from_value(json!({
        "title": title,
        "description": null,
        "system_prompt": "You coach the group.",
    }))
    .unwrap();
    let coach = resources
        .common
        .repos
        .coaches
        .create(user_id, tenant_id, &request)
        .await
        .unwrap();
    let coach_id = coach.id.to_string();
    resources
        .common
        .repos
        .tenants
        .set_selected_coach(tenant_id, user_id, Some(&coach_id))
        .await
        .unwrap();
    coach_id
}

async fn create_conversation(router: axum::Router, auth: &str) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", auth)
        .json(&json!({"title": "Cmd Test", "model": MODEL}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let conv: ConversationResponse = resp.json();
    conv.id
}

/// Send one slash command on a conversation and return the turn, asserting
/// it was answered as a command rather than handed to the LLM.
async fn send(router: axum::Router, auth: &str, conv_id: &str, text: &str) -> TurnResponse {
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/messages"))
        .header("authorization", auth)
        .json(&json!({"content": text}))
        .send(router)
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK, "`{text}` failed");
    let body: TurnResponse = resp.json();
    assert_eq!(
        body.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "`{text}` must be answered as a command"
    );
    body
}

/// The controls a turn carries, or none.
fn actions(body: &TurnResponse) -> Vec<&ChatMessageAction> {
    body.assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlockResponse::Actions { actions, .. } => Some(actions.iter().collect()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The label above a turn's controls, when it carried one.
fn actions_title(body: &TurnResponse) -> Option<&str> {
    body.assistant.blocks.iter().find_map(|block| match block {
        ReplyBlockResponse::Actions { title, .. } => title.as_deref(),
        _ => None,
    })
}

/// The postback values of every install button on a `/discover` card.
fn install_postbacks(body: &TurnResponse) -> Vec<String> {
    actions(body)
        .iter()
        .filter(|a| a.value.starts_with("/discover install @"))
        .map(|a| a.value.clone())
        .collect()
}

/// The reply a test user reads: every seeded user keeps the default locale,
/// so the expected text is the registry's own rendering for it.
fn rendered(resources: &Arc<ServerContext>, key: &str, args: &[&str]) -> String {
    let locale = default_locale();
    resources
        .mcp
        .messaging_strings_registry
        .render(key, &locale, args)
}

/// The catalogue handle a published coach was assigned.
async fn handle_of(resources: &Arc<ServerContext>, coach_id: Uuid) -> String {
    resources
        .common
        .repos
        .store_listings
        .get_published_coach(&coach_id.to_string())
        .await
        .unwrap()
        .expect("published")
        .coach
        .handle
        .expect("a published coach owns a handle")
}

// ============================================================================
// /discover
// ============================================================================

#[tokio::test]
async fn discover_pages_the_catalogue_eight_at_a_time_with_install_buttons() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author_tenant, _) =
        seed_user_tenant(&resources, "discover-author@test.com", "professional").await;
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "discover-list@test.com", "professional").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;

    // Nothing published yet: the bare command says so, with no buttons.
    let empty = send(router.clone(), &auth, &conv, "/discover").await;
    assert_eq!(
        empty.assistant.message.content,
        rendered(&resources, KEY_DISCOVER_CATALOGUE_EMPTY, &[])
    );
    assert!(actions(&empty).is_empty());

    let mut published = Vec::new();
    for n in 1..=9 {
        let id = publish_catalogue_coach(
            &resources.common.repos,
            author_id,
            author_tenant,
            &format!("Shelf Coach {n}"),
            "You are a shelf coach.",
        )
        .await;
        published.push(format!(
            "/discover install @{}",
            handle_of(&resources, id).await
        ));
    }

    let page = send(router.clone(), &auth, &conv, "/discover").await;
    assert_eq!(
        actions_title(&page),
        Some(rendered(&resources, KEY_DISCOVER_CARD_TITLE, &[]).as_str())
    );
    let first = install_postbacks(&page);
    assert_eq!(first.len(), 8, "eight coaches per card");
    for postback in &first {
        assert!(published.contains(postback), "unknown button {postback}");
        assert!(
            postback.len() <= TELEGRAM_CALLBACK_LIMIT,
            "{postback} exceeds Telegram's callback budget"
        );
        let handle = postback.trim_start_matches("/discover install @");
        assert!(
            page.assistant
                .message
                .content
                .contains(&format!("@{handle}")),
            "the card names @{handle}: {}",
            page.assistant.message.content
        );
    }
    let more = actions(&page)
        .into_iter()
        .find(|a| a.value.starts_with("/discover more"))
        .expect("a ninth coach means a More button");
    assert_eq!(more.value, "/discover more 8");
    assert_eq!(
        more.label,
        rendered(&resources, KEY_DISCOVER_MORE_LABEL, &[])
    );
    assert_eq!(actions(&page).len(), 9, "eight installs and one More");

    // The More button sends the next page: the one coach left, and no More.
    let next = send(router, &auth, &conv, &more.value).await;
    let second = install_postbacks(&next);
    assert_eq!(second.len(), 1, "one coach on the second page");
    assert!(
        !first.contains(&second[0]),
        "the second page holds the coach the first left out"
    );
    assert!(
        !actions(&next)
            .iter()
            .any(|a| a.value.starts_with("/discover more")),
        "the last page carries no More button"
    );
    let mut seen: Vec<String> = first.into_iter().chain(second).collect();
    seen.sort();
    published.sort();
    assert_eq!(
        seen, published,
        "both pages together are the whole catalogue"
    );
}

#[tokio::test]
async fn discover_filters_one_category_case_insensitively() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author_tenant, _) =
        seed_user_tenant(&resources, "category-author@test.com", "professional").await;
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "category-list@test.com", "professional").await;
    let repos = &resources.common.repos;
    for title in ["Base Miles Coach", "Threshold Coach"] {
        publish_catalogue_coach_in(
            repos,
            author_id,
            author_tenant,
            title,
            "You train.",
            CoachCategory::Training,
        )
        .await;
    }
    let fuel = publish_catalogue_coach_in(
        repos,
        author_id,
        author_tenant,
        "Fuel Coach",
        "You feed.",
        CoachCategory::Nutrition,
    )
    .await;
    let fuel_handle = handle_of(&resources, fuel).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;

    let nutrition = send(router.clone(), &auth, &conv, "/discover NUTRITION").await;
    assert_eq!(
        install_postbacks(&nutrition),
        vec![format!("/discover install @{fuel_handle}")],
        "the Nutrition shelf holds exactly the fuel coach"
    );
    assert!(nutrition.assistant.message.content.contains("Fuel Coach"));
    assert!(!nutrition
        .assistant
        .message
        .content
        .contains("Threshold Coach"));

    let training = send(router, &auth, &conv, "/discover training").await;
    assert_eq!(
        install_postbacks(&training).len(),
        2,
        "both training coaches"
    );
    assert!(!training.assistant.message.content.contains("Fuel Coach"));
}

#[tokio::test]
async fn discover_searches_when_the_words_are_not_a_category() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author_tenant, _) =
        seed_user_tenant(&resources, "search-author@test.com", "professional").await;
    let (_user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "search-list@test.com", "professional").await;
    let repos = &resources.common.repos;
    let taper = publish_catalogue_coach(
        repos,
        author_id,
        author_tenant,
        "Marathon Taper Coach",
        "You taper.",
    )
    .await;
    publish_catalogue_coach(
        repos,
        author_id,
        author_tenant,
        "Hill Repeats Coach",
        "You climb.",
    )
    .await;
    let taper_handle = handle_of(&resources, taper).await;

    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;

    let hit = send(router.clone(), &auth, &conv, "/discover marathon taper").await;
    assert_eq!(
        install_postbacks(&hit),
        vec![format!("/discover install @{taper_handle}")],
        "a multi-word search matches on the title"
    );
    assert!(
        !actions(&hit)
            .iter()
            .any(|a| a.value.starts_with("/discover more")),
        "a search is never paged"
    );

    let miss = send(router, &auth, &conv, "/discover nothing like this").await;
    assert_eq!(
        miss.assistant.message.content,
        rendered(&resources, KEY_DISCOVER_EMPTY, &["nothing like this"])
    );
    assert!(actions(&miss).is_empty());
}

// ============================================================================
// /discover install
// ============================================================================

#[tokio::test]
async fn discover_install_by_handle_installs_once_and_teaches_coach_add() {
    let resources = create_test_server_resources().await.unwrap();
    let (author_id, author_tenant, _) =
        seed_user_tenant(&resources, "install-author@test.com", "professional").await;
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "install-user@test.com", "professional").await;
    let origin = publish_catalogue_coach(
        &resources.common.repos,
        author_id,
        author_tenant,
        "Recovery Coach",
        "You are the recovery coach.",
    )
    .await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;
    let (events, _guard) = capture_notify();

    let usage = send(router.clone(), &auth, &conv, "/discover install").await;
    assert_eq!(
        usage.assistant.message.content,
        rendered(&resources, KEY_DISCOVER_INSTALL_USAGE, &[])
    );
    let unknown = send(
        router.clone(),
        &auth,
        &conv,
        "/discover install @nobody-here",
    )
    .await;
    assert_eq!(
        unknown.assistant.message.content,
        rendered(
            &resources,
            KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE,
            &["@nobody-here"]
        )
    );
    assert!(actions(&unknown).is_empty());
    assert!(
        named(&events, "coach.installed").is_empty(),
        "nothing was installed"
    );

    let installed = send(
        router.clone(),
        &auth,
        &conv,
        "/discover install @recovery-coach",
    )
    .await;
    assert_eq!(actions_title(&installed), Some("Recovery Coach"));
    assert_eq!(
        installed.assistant.message.content,
        rendered(
            &resources,
            KEY_DISCOVER_INSTALLED,
            &["Recovery Coach", "recovery-coach"]
        )
    );
    let buttons = actions(&installed);
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0].action_type, "postback");
    assert_eq!(buttons[0].value, "/coach add @recovery-coach");
    assert_eq!(
        buttons[0].label,
        rendered(&resources, KEY_DISCOVER_ADD_LABEL, &[])
    );

    let handle = CoachHandle::parse("recovery-coach").unwrap();
    let copy = resources
        .common
        .repos
        .coaches
        .find_installed_by_handle(&handle, user_id, tenant_id)
        .await
        .unwrap()
        .expect("the athlete's copy resolves by handle");
    assert_eq!(copy.forked_from, Some(origin));
    assert_eq!(copy.title, "Recovery Coach");
    let emitted = only(&events, "coach.installed");
    assert_eq!(emitted.field("user_id"), user_id.to_string());
    assert_eq!(emitted.field("tenant_id"), tenant_id.to_string());
    assert_eq!(emitted.field("coach_slug"), origin.to_string());

    // A second install is the same hint and no second copy.
    let again = send(router, &auth, &conv, "/discover install @recovery-coach").await;
    assert_eq!(
        again.assistant.message.content,
        rendered(
            &resources,
            KEY_DISCOVER_INSTALL_ALREADY,
            &["Recovery Coach", "recovery-coach"]
        )
    );
    assert_eq!(actions(&again)[0].value, "/coach add @recovery-coach");
    let library = resources
        .common
        .repos
        .store_listings
        .get_installed_coaches(user_id, tenant_id)
        .await
        .unwrap();
    assert_eq!(
        library.len(),
        1,
        "one copy, however many times it is asked for"
    );
    assert_eq!(named(&events, "coach.installed").len(), 1, "counted once");
}

// ============================================================================
// /group create
// ============================================================================

#[tokio::test]
async fn group_create_in_a_fresh_thread_binds_it_to_the_new_group() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "group-create@test.com", "professional").await;
    let coach_id = seed_selected_coach(&resources, user_id, tenant_id, "Club Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;
    let (events, _guard) = capture_notify();

    let reply = send(router.clone(), &auth, &conv, "/group create Sunday Runners").await;
    assert_eq!(actions_title(&reply), Some("Sunday Runners"));
    assert_eq!(
        reply.assistant.message.content,
        rendered(
            &resources,
            KEY_GROUP_CREATED,
            &["Sunday Runners", "Club Coach"]
        )
    );
    let buttons = actions(&reply);
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0].value, "/group invite");
    assert_eq!(
        buttons[0].label,
        rendered(&resources, KEY_GROUP_INVITE_LABEL, &[])
    );

    let repos = &resources.common.repos;
    let thread = repos
        .chat
        .get_conversation(&conv, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    let group_id = thread
        .group_id
        .clone()
        .expect("the empty thread became the group chat");
    assert_eq!(thread.coach_id.as_deref(), Some(coach_id.as_str()));
    assert_eq!(thread.title, "Sunday Runners");
    let group = repos
        .groups
        .get_group(&group_id, tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.coach_id, coach_id);
    let owner = repos
        .groups
        .get_member(&group_id, user_id)
        .await
        .unwrap()
        .expect("the creator is a member");
    assert_eq!(owner.role, GroupRole::Owner);
    let created = only(&events, "group.created");
    assert_eq!(created.field("group_id"), group_id);

    // The thread now has a group, so a second create files a new group
    // conversation beside it and leaves this one bound where it was.
    send(router, &auth, &conv, "/group create Second Wind").await;
    let listed = repos
        .chat
        .list_conversations(&user_id.to_string(), tenant_id, 50, 0)
        .await
        .unwrap()
        .items;
    let second = listed
        .iter()
        .find(|c| c.title == "Second Wind")
        .expect("the second group got its own conversation");
    let second_record = repos
        .chat
        .get_conversation(&second.id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert!(second_record.group_id.is_some_and(|g| g != group_id));
    assert_eq!(second_record.coach_id.as_deref(), Some(coach_id.as_str()));
    let unchanged = repos
        .chat
        .get_conversation(&conv, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unchanged.group_id.as_deref(), Some(group_id.as_str()));
    assert_eq!(named(&events, "group.created").len(), 2);
}

#[tokio::test]
async fn group_create_in_a_thread_with_history_files_a_group_conversation_beside_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "group-history@test.com", "professional").await;
    let coach_id = seed_selected_coach(&resources, user_id, tenant_id, "Evening Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;
    let repos = &resources.common.repos;
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: &conv,
            user_id: &user_id.to_string(),
            role: "user",
            content: "How was my week?",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
        })
        .await
        .unwrap();

    send(router, &auth, &conv, "/group create Evening Club").await;

    let thread = repos
        .chat
        .get_conversation(&conv, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(thread.group_id, None, "a thread with history is left alone");
    assert_eq!(thread.title, "Cmd Test");
    let listed = repos
        .chat
        .list_conversations(&user_id.to_string(), tenant_id, 50, 0)
        .await
        .unwrap()
        .items;
    let club = listed
        .iter()
        .find(|c| c.title == "Evening Club")
        .expect("the group got its own conversation");
    let record = repos
        .chat
        .get_conversation(&club.id, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    let group = repos
        .groups
        .get_group(record.group_id.as_deref().unwrap(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.name, "Evening Club");
    assert_eq!(group.coach_id, coach_id);
    assert_eq!(record.coach_id.as_deref(), Some(coach_id.as_str()));
    assert_eq!(
        record.model, MODEL,
        "the group conversation inherits the thread's model"
    );
}

#[tokio::test]
async fn group_create_refuses_without_a_name_or_a_coach() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _tenant_id, auth) =
        seed_user_tenant(&resources, "group-refuse@test.com", "professional").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;

    let bare = send(router.clone(), &auth, &conv, "/group create").await;
    assert_eq!(
        bare.assistant.message.content,
        rendered(&resources, KEY_GROUP_CREATE_USAGE, &[])
    );
    let no_coach = send(router, &auth, &conv, "/group create Nameless Riders").await;
    assert_eq!(
        no_coach.assistant.message.content,
        rendered(&resources, KEY_GROUP_CREATE_NO_COACH, &[])
    );
    assert!(actions(&no_coach).is_empty());
    let groups = resources
        .common
        .repos
        .groups
        .list_groups_for_user(user_id)
        .await
        .unwrap();
    assert!(groups.is_empty(), "no group without a coach");
}

#[tokio::test]
async fn group_create_on_a_starter_tenant_applies_the_starter_cap_like_the_rest_route() {
    // POST /api/groups resolves the member cap from the tenant plan and hands
    // it to the service, which refuses only a cap of zero. Starter's cap is a
    // real number, so a Starter owner creates a group there — and here.
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant_id, auth) =
        seed_user_tenant(&resources, "group-starter@test.com", "starter").await;
    seed_selected_coach(&resources, user_id, tenant_id, "Starter Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &auth).await;

    let reply = send(router, &auth, &conv, "/group create Starter Squad").await;
    assert_eq!(actions_title(&reply), Some("Starter Squad"));

    let repos = &resources.common.repos;
    let thread = repos
        .chat
        .get_conversation(&conv, &user_id.to_string(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    let group = repos
        .groups
        .get_group(thread.group_id.as_deref().unwrap(), tenant_id)
        .await
        .unwrap()
        .unwrap();
    let starter_cap = i32::try_from(tier_strategy_for("starter").max_members_per_group()).unwrap();
    assert_eq!(
        group.max_members, starter_cap,
        "clamped to the Starter tier"
    );
}

#[tokio::test]
async fn group_create_is_refused_by_the_policy_for_a_plain_member_until_the_tenant_opens_it() {
    let resources = create_test_server_resources().await.unwrap();
    let (owner_id, tenant_id, _owner_auth) =
        seed_user_tenant(&resources, "policy-owner@test.com", "professional").await;
    let (member_id, member_auth) =
        seed_tenant_member(&resources, "policy-member@test.com", tenant_id, false).await;
    seed_selected_coach(&resources, member_id, tenant_id, "Member Coach").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &member_auth).await;
    let repos = &resources.common.repos;

    // The default policy reserves creation to the tenant's admins.
    let refused = send(
        router.clone(),
        &member_auth,
        &conv,
        "/group create Members Club",
    )
    .await;
    assert_eq!(
        refused.assistant.message.content,
        rendered(&resources, KEY_GROUP_CREATE_FORBIDDEN, &[])
    );
    assert!(actions(&refused).is_empty());
    assert!(repos
        .groups
        .list_groups_for_user(member_id)
        .await
        .unwrap()
        .is_empty());

    // The tenant opens creation to everyone — the same admin-config key the
    // REST create route and GET /api/groups/permissions read.
    let tenant = tenant_id.to_string();
    let owner = owner_id.to_string();
    let everyone = json!("everyone");
    AdminConfigManager::new(resources.coach.database.sqlite_pool().unwrap().clone())
        .set_override(SetOverrideParams {
            category: GROUP_PERMISSIONS_CATEGORY,
            key: GROUP_CREATION_POLICY_KEY,
            value: &everyone,
            data_type: ConfigDataType::Enum,
            admin_user_id: &owner,
            scope: ConfigScope::Tenant(&tenant),
            reason: Some("open group creation to every member"),
        })
        .await
        .unwrap();

    let created = send(router, &member_auth, &conv, "/group create Members Club").await;
    assert_eq!(actions_title(&created), Some("Members Club"));
    let groups = repos.groups.list_groups_for_user(member_id).await.unwrap();
    assert_eq!(groups.len(), 1);
    let group = repos
        .groups
        .get_group(&groups[0].id.to_string(), tenant_id)
        .await
        .unwrap()
        .expect("filed under the member's tenant");
    assert_eq!(group.owner_id, member_id);
}

// ============================================================================
// /group join
// ============================================================================

/// A group owned by `owner_id` with one open invite of `kind`; returns the
/// group id and the invite code.
async fn seed_group_with_invite(
    resources: &Arc<ServerContext>,
    owner_id: Uuid,
    tenant_id: TenantId,
    coach_id: &str,
    kind: GroupInviteKind,
) -> (Uuid, String) {
    let request = CreateGroupRequest {
        name: "Trail Crew".to_owned(),
        description: None,
        coach_id: coach_id.to_owned(),
        max_members: None,
    };
    let group = resources
        .group_service()
        .create_group(&request, owner_id, tenant_id, 10)
        .await
        .unwrap();
    let invite = resources
        .group_service()
        .create_invite(group.id, owner_id, tenant_id, None, None, kind)
        .await
        .unwrap();
    (group.id, invite.code)
}

#[tokio::test]
async fn group_join_by_code_adds_the_member_and_files_their_group_conversation() {
    let resources = create_test_server_resources().await.unwrap();
    let (owner_id, owner_tenant, _owner_auth) =
        seed_user_tenant(&resources, "join-owner@test.com", "professional").await;
    let coach_id = seed_selected_coach(&resources, owner_id, owner_tenant, "Trail Coach").await;
    let (group_id, code) = seed_group_with_invite(
        &resources,
        owner_id,
        owner_tenant,
        &coach_id,
        GroupInviteKind::Member,
    )
    .await;
    let (member_id, member_tenant, member_auth) =
        seed_user_tenant(&resources, "join-member@test.com", "professional").await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let conv = create_conversation(router.clone(), &member_auth).await;
    let repos = &resources.common.repos;
    let (events, _guard) = capture_notify();

    let joined = send(
        router.clone(),
        &member_auth,
        &conv,
        &format!("/group join {code}"),
    )
    .await;
    assert_eq!(
        joined.assistant.message.content,
        rendered(&resources, KEY_GROUP_JOINED, &["Trail Crew"])
    );
    let member = repos
        .groups
        .get_member(&group_id.to_string(), member_id)
        .await
        .unwrap()
        .expect("joined as a member");
    assert_eq!(member.role, GroupRole::Member);
    let joined_event = only(&events, "group.joined");
    assert_eq!(joined_event.field("group_id"), group_id.to_string());
    assert_eq!(joined_event.field("tenant_id"), owner_tenant.to_string());

    // The group is now in the member's own list, as their group-scoped
    // conversation under their own tenant.
    let listed = repos
        .chat
        .list_conversations(&member_id.to_string(), member_tenant, 50, 0)
        .await
        .unwrap()
        .items;
    let rows: Vec<_> = listed.iter().filter(|c| c.title == "Trail Crew").collect();
    assert_eq!(rows.len(), 1, "one group conversation for the member");
    let record = repos
        .chat
        .get_conversation(&rows[0].id, &member_id.to_string(), member_tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.group_id.as_deref(),
        Some(group_id.to_string().as_str())
    );
    assert_eq!(record.coach_id.as_deref(), Some(coach_id.as_str()));
    assert_eq!(record.model, MODEL);

    // Joining twice names the group and files nothing more.
    let again = send(
        router.clone(),
        &member_auth,
        &conv,
        &format!("/group join {code}"),
    )
    .await;
    assert_eq!(
        again.assistant.message.content,
        rendered(&resources, KEY_GROUP_JOIN_ALREADY_MEMBER, &["Trail Crew"])
    );
    let listed = repos
        .chat
        .list_conversations(&member_id.to_string(), member_tenant, 50, 0)
        .await
        .unwrap()
        .items;
    assert_eq!(listed.iter().filter(|c| c.title == "Trail Crew").count(), 1);
    assert_eq!(named(&events, "group.joined").len(), 1);

    // An unusable code gets the one fixed refusal, which never echoes it.
    for text in ["/group join", "/group join ZZZZ9999"] {
        let refused = send(router.clone(), &member_auth, &conv, text).await;
        assert_eq!(
            refused.assistant.message.content,
            rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[])
        );
        assert!(!refused.assistant.message.content.contains("ZZZZ9999"));
    }
}

#[tokio::test]
async fn group_join_with_a_coach_invite_attaches_an_eligible_roster_coach_only() {
    let resources = create_test_server_resources().await.unwrap();
    let (owner_id, owner_tenant, _owner_auth) =
        seed_user_tenant(&resources, "coachjoin-owner@test.com", "professional").await;
    let coach_id = seed_selected_coach(&resources, owner_id, owner_tenant, "Trail Coach").await;
    let (group_id, code) = seed_group_with_invite(
        &resources,
        owner_id,
        owner_tenant,
        &coach_id,
        GroupInviteKind::Coach,
    )
    .await;
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let repos = &resources.common.repos;

    // An athlete from another tenant is not eligible: the code reads as unusable.
    let (_outsider_id, _outsider_tenant, outsider_auth) =
        seed_user_tenant(&resources, "coachjoin-outsider@test.com", "professional").await;
    let outsider_conv = create_conversation(router.clone(), &outsider_auth).await;
    let refused = send(
        router.clone(),
        &outsider_auth,
        &outsider_conv,
        &format!("/group join {code}"),
    )
    .await;
    assert_eq!(
        refused.assistant.message.content,
        rendered(&resources, KEY_GROUP_JOIN_INVALID_CODE, &[])
    );
    assert!(!refused.assistant.message.content.contains(&code));
    let untouched = repos
        .groups
        .get_group(&group_id.to_string(), owner_tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.coach_user_id, None);

    // A roster-managing coach in the group's tenant is attached, not enrolled.
    let (coach_user_id, coach_auth) =
        seed_tenant_member(&resources, "coachjoin-coach@test.com", owner_tenant, true).await;
    let coach_conv = create_conversation(router.clone(), &coach_auth).await;
    let attached = send(
        router,
        &coach_auth,
        &coach_conv,
        &format!("/group join {code}"),
    )
    .await;
    assert_eq!(
        attached.assistant.message.content,
        rendered(&resources, KEY_GROUP_JOINED_AS_COACH, &["Trail Crew"])
    );
    let group = repos
        .groups
        .get_group(&group_id.to_string(), owner_tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(group.coach_user_id, Some(coach_user_id));
    assert!(repos
        .groups
        .get_member(&group_id.to_string(), coach_user_id)
        .await
        .unwrap()
        .is_none());
    let listed = repos
        .chat
        .list_conversations(&coach_user_id.to_string(), owner_tenant, 50, 0)
        .await
        .unwrap()
        .items;
    assert!(
        !listed.iter().any(|c| c.title == "Trail Crew"),
        "a human coach gets no member conversation"
    );
}
