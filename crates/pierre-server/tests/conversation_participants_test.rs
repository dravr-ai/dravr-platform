// ABOUTME: Integration tests for conversation participants over REST
// ABOUTME: Owner auto-row, member read/post/rename, stranger 404s, removal revokes, cross-tenant 403, backfill

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;
use sqlx::migrate::Migrator;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;
use uuid::Uuid;

use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::ParticipantRole;
use pierre_mcp_server::routes::chat::{
    ChatRoutes, ConversationListResponse, ConversationResponse, MessagesListResponse,
    ParticipantListResponse, ParticipantResponse, TurnResponse,
};

/// Migrations of the `SQLite` lane, applied by hand for the backfill test.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Version of the migration under test, as named in `migrations/`.
const PARTICIPANTS_MIGRATION: i64 = 20_260_826_000_004;

struct Fixture {
    router: axum::Router,
    /// Opens the conversation; the owner participant.
    owner_auth: String,
    /// Same tenant as the owner; gets added to the thread.
    member_id: Uuid,
    member_auth: String,
    /// Same tenant as the owner; never added.
    stranger_id: Uuid,
    stranger_auth: String,
    /// A real user in a *different* tenant.
    foreigner_id: Uuid,
}

/// Three users in the owner's tenant plus one outside it. The same-tenant
/// users get a real `tenant_users` row and a token scoped to that tenant, so
/// every refusal under test is a membership decision, never an auth one.
async fn setup() -> Fixture {
    let res = create_test_server_resources().await.unwrap();
    let repos = res.coach.database.repositories();

    let (owner_id, owner, _) =
        create_test_user_with_plan(&res.coach.database, "part-owner@test.com", "professional")
            .await
            .unwrap();
    let (member_id, member, _) =
        create_test_user_with_plan(&res.coach.database, "part-member@test.com", "professional")
            .await
            .unwrap();
    let (stranger_id, stranger, _) = create_test_user_with_plan(
        &res.coach.database,
        "part-stranger@test.com",
        "professional",
    )
    .await
    .unwrap();
    let (foreigner_id, _, _) = create_test_user_with_plan(
        &res.coach.database,
        "part-foreigner@test.com",
        "professional",
    )
    .await
    .unwrap();

    let shared_tid = repos
        .tenants
        .list_for_user(owner_id)
        .await
        .unwrap()
        .first()
        .unwrap()
        .id;
    for id in [member_id, stranger_id] {
        repos.users.update_tenant_id(id, shared_tid).await.unwrap();
    }

    let tenant_token = |user| {
        format!(
            "Bearer {}",
            res.auth
                .auth_manager
                .generate_token_with_tenant(
                    user,
                    &res.auth.jwks_manager,
                    Some(shared_tid.to_string()),
                )
                .unwrap()
        )
    };
    let member_auth = tenant_token(&member);
    let stranger_auth = tenant_token(&stranger);
    let owner_auth = format!("Bearer {}", generate_test_token(&res, &owner).await);

    let router = ChatRoutes::routes(Arc::clone(&res));

    Fixture {
        router,
        owner_auth,
        member_id,
        member_auth,
        stranger_id,
        stranger_auth,
        foreigner_id,
    }
}

async fn create_conversation(fx: &Fixture) -> ConversationResponse {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "title": "Shared thread" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    resp.json()
}

async fn add_participant(fx: &Fixture, conv_id: &str, auth: &str, user_id: Uuid) -> StatusCode {
    AxumTestRequest::post(&format!("/api/chat/conversations/{conv_id}/participants"))
        .header("authorization", auth)
        .json(&json!({ "user_id": user_id.to_string() }))
        .send(fx.router.clone())
        .await
        .status_code()
}

async fn list_participants(fx: &Fixture, conv_id: &str, auth: &str) -> Vec<ParticipantResponse> {
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{conv_id}/participants"))
        .header("authorization", auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    resp.json::<ParticipantListResponse>().participants
}

async fn get_status(fx: &Fixture, path: &str, auth: &str) -> StatusCode {
    AxumTestRequest::get(path)
        .header("authorization", auth)
        .send(fx.router.clone())
        .await
        .status_code()
}

#[tokio::test]
async fn test_owner_is_written_as_a_participant_on_create() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;

    let participants = list_participants(&fx, &conv.id, &fx.owner_auth).await;
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].role, ParticipantRole::Owner);
    assert_eq!(participants[0].user_id, participants[0].added_by);
    assert!(!participants[0].added_at.is_empty());
}

#[tokio::test]
async fn test_added_member_reads_lists_renames_and_posts() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;

    // Before the add, the same-tenant member is a stranger to the thread.
    assert_eq!(
        get_status(
            &fx,
            &format!("/api/chat/conversations/{}", conv.id),
            &fx.member_auth
        )
        .await,
        StatusCode::NOT_FOUND
    );

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{}/participants", conv.id))
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "user_id": fx.member_id.to_string() }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::CREATED);
    let added: ParticipantResponse = resp.json();
    assert_eq!(added.user_id, fx.member_id.to_string());
    assert_eq!(added.role, ParticipantRole::Member);
    assert_ne!(added.added_by, added.user_id, "added_by names the owner");

    // Read the conversation.
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{}", conv.id))
        .header("authorization", &fx.member_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(resp.json::<ConversationResponse>().id, conv.id);

    // It shows up in the member's own listing.
    let resp = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &fx.member_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let listing: ConversationListResponse = resp.json();
    assert_eq!(listing.total, 1);
    assert_eq!(listing.conversations[0].id, conv.id);

    // Post in it: a slash command is answered by the platform without an LLM,
    // and goes through the same send-message membership gate as any turn.
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{}/messages", conv.id))
        .header("authorization", &fx.member_auth)
        .json(&json!({ "content": "/help" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let turn: TurnResponse = resp.json();
    assert_eq!(turn.assistant.finish_reason.as_deref(), Some("command"));
    assert!(!turn.assistant.message.content.is_empty());

    // Read the messages and rename the thread.
    let resp = AxumTestRequest::get(&format!("/api/chat/conversations/{}/messages", conv.id))
        .header("authorization", &fx.member_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    let _: MessagesListResponse = resp.json();

    let resp = AxumTestRequest::put(&format!("/api/chat/conversations/{}", conv.id))
        .header("authorization", &fx.member_auth)
        .json(&json!({ "title": "Renamed by the member" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::OK);
    assert_eq!(
        resp.json::<ConversationResponse>().title,
        "Renamed by the member"
    );

    // The member sees both participants, owner first.
    let participants = list_participants(&fx, &conv.id, &fx.member_auth).await;
    assert_eq!(participants.len(), 2);
    assert_eq!(participants[0].role, ParticipantRole::Owner);
    assert_eq!(participants[1].user_id, fx.member_id.to_string());
}

#[tokio::test]
async fn test_stranger_in_the_same_tenant_gets_404_everywhere() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;
    let base = format!("/api/chat/conversations/{}", conv.id);

    assert_eq!(
        get_status(&fx, &base, &fx.stranger_auth).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_status(&fx, &format!("{base}/messages"), &fx.stranger_auth).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_status(&fx, &format!("{base}/participants"), &fx.stranger_auth).await,
        StatusCode::NOT_FOUND
    );

    let post = AxumTestRequest::post(&format!("{base}/messages"))
        .header("authorization", &fx.stranger_auth)
        .json(&json!({ "content": "/help" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(post.status_code(), StatusCode::NOT_FOUND);

    let rename = AxumTestRequest::put(&base)
        .header("authorization", &fx.stranger_auth)
        .json(&json!({ "title": "Hijacked" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(rename.status_code(), StatusCode::NOT_FOUND);

    // A stranger cannot let themself in.
    assert_eq!(
        add_participant(&fx, &conv.id, &fx.stranger_auth, fx.stranger_id).await,
        StatusCode::NOT_FOUND
    );
    let delete = AxumTestRequest::delete(&base)
        .header("authorization", &fx.stranger_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(delete.status_code(), StatusCode::NOT_FOUND);

    // And their listing stays empty.
    let resp = AxumTestRequest::get("/api/chat/conversations")
        .header("authorization", &fx.stranger_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.json::<ConversationListResponse>().total, 0);

    // The thread is untouched by all of the above.
    let participants = list_participants(&fx, &conv.id, &fx.owner_auth).await;
    assert_eq!(participants.len(), 1);
}

#[tokio::test]
async fn test_removing_a_member_revokes_access() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;
    assert_eq!(
        add_participant(&fx, &conv.id, &fx.owner_auth, fx.member_id).await,
        StatusCode::CREATED
    );
    let base = format!("/api/chat/conversations/{}", conv.id);
    assert_eq!(
        get_status(&fx, &base, &fx.member_auth).await,
        StatusCode::OK
    );

    let resp = AxumTestRequest::delete(&format!("{base}/participants/{}", fx.member_id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);

    assert_eq!(
        get_status(&fx, &base, &fx.member_auth).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get_status(&fx, &format!("{base}/messages"), &fx.member_auth).await,
        StatusCode::NOT_FOUND
    );
    let participants = list_participants(&fx, &conv.id, &fx.owner_auth).await;
    assert_eq!(participants.len(), 1);
    assert_eq!(participants[0].role, ParticipantRole::Owner);

    // Removing them again: nothing left to remove.
    let resp = AxumTestRequest::delete(&format!("{base}/participants/{}", fx.member_id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_member_can_add_and_remove_others_but_not_the_owner() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;
    assert_eq!(
        add_participant(&fx, &conv.id, &fx.owner_auth, fx.member_id).await,
        StatusCode::CREATED
    );

    // A member adds the stranger in.
    assert_eq!(
        add_participant(&fx, &conv.id, &fx.member_auth, fx.stranger_id).await,
        StatusCode::CREATED
    );
    assert_eq!(
        get_status(
            &fx,
            &format!("/api/chat/conversations/{}", conv.id),
            &fx.stranger_auth
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        list_participants(&fx, &conv.id, &fx.member_auth)
            .await
            .len(),
        3
    );

    // Re-adding is idempotent.
    assert_eq!(
        add_participant(&fx, &conv.id, &fx.member_auth, fx.stranger_id).await,
        StatusCode::CREATED
    );
    assert_eq!(
        list_participants(&fx, &conv.id, &fx.member_auth)
            .await
            .len(),
        3
    );

    // The owner cannot be removed by anyone.
    let owner_id = list_participants(&fx, &conv.id, &fx.owner_auth).await[0]
        .user_id
        .clone();
    let resp = AxumTestRequest::delete(&format!(
        "/api/chat/conversations/{}/participants/{owner_id}",
        conv.id
    ))
    .header("authorization", &fx.member_auth)
    .send(fx.router.clone())
    .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
    assert_eq!(
        list_participants(&fx, &conv.id, &fx.owner_auth).await.len(),
        3
    );

    // Only the owner deletes the thread: a member is refused, and told why.
    let resp = AxumTestRequest::delete(&format!("/api/chat/conversations/{}", conv.id))
        .header("authorization", &fx.member_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);
    let resp = AxumTestRequest::delete(&format!("/api/chat/conversations/{}", conv.id))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_cross_tenant_add_is_refused_explicitly() {
    let fx = setup().await;
    let conv = create_conversation(&fx).await;

    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{}/participants", conv.id))
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "user_id": fx.foreigner_id.to_string() }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json();
    assert_eq!(body["code"], "PermissionDenied", "{body}");

    // Nothing was written.
    assert_eq!(
        list_participants(&fx, &conv.id, &fx.owner_auth).await.len(),
        1
    );

    // A malformed id is a 400, not a 403 or a 500.
    let resp = AxumTestRequest::post(&format!("/api/chat/conversations/{}/participants", conv.id))
        .header("authorization", &fx.owner_auth)
        .json(&json!({ "user_id": "not-a-uuid" }))
        .send(fx.router.clone())
        .await;
    assert_eq!(resp.status_code(), StatusCode::BAD_REQUEST);
}

/// Apply every migration before the participants one, plant a conversation
/// the way the pre-participants schema wrote it, then apply the participants
/// migration and assert the owner row was backfilled.
#[tokio::test]
async fn test_migration_backfills_an_owner_row_for_every_existing_conversation() {
    // The planted rows name users that do not exist: this test is about the
    // schema step, not the users table, and sqlx enables FK enforcement on
    // its `SQLite` connections by default.
    // One connection: every pooled connection to an in-memory database is
    // its own empty database, so a second one would see none of the schema.
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();

    let mut applied_target = false;
    for migration in MIGRATOR.iter() {
        if migration.version == PARTICIPANTS_MIGRATION {
            // Two conversations from two tenants, both without a participant row.
            for (id, user, tenant) in [
                ("conv-a", "user-a", "tenant-a"),
                ("conv-b", "user-b", "tenant-b"),
            ] {
                sqlx::query(
                    "INSERT INTO chat_conversations (id, user_id, tenant_id, title, model, total_tokens, created_at, updated_at)
                     VALUES ($1, $2, $3, 'legacy', 'm', 0, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                )
                .bind(id)
                .bind(user)
                .bind(tenant)
                .execute(&pool)
                .await
                .unwrap();
            }
            sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
            applied_target = true;
            break;
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    assert!(
        applied_target,
        "the participants migration is in migrations/"
    );

    let rows = sqlx::query(
        "SELECT conversation_id, user_id, tenant_id, role, added_by, added_at
         FROM conversation_participants ORDER BY conversation_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("conversation_id"), "conv-a");
    assert_eq!(rows[0].get::<String, _>("user_id"), "user-a");
    assert_eq!(rows[0].get::<String, _>("tenant_id"), "tenant-a");
    assert_eq!(rows[0].get::<String, _>("role"), "owner");
    assert_eq!(rows[0].get::<String, _>("added_by"), "user-a");
    assert_eq!(rows[0].get::<String, _>("added_at"), "2026-01-01T00:00:00Z");
    assert_eq!(rows[1].get::<String, _>("user_id"), "user-b");
    assert_eq!(rows[1].get::<String, _>("tenant_id"), "tenant-b");
}
