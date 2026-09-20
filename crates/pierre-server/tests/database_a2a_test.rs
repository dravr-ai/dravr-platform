// ABOUTME: Unit tests for database a2a functionality
// ABOUTME: Validates database a2a behavior, edge cases, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use chrono::Utc;
use pierre_auth::api_keys::{ApiKey, ApiKeyTier};
use pierre_core::models::a2a::{A2APushNotificationConfig, A2AUsage};
use pierre_core::models::CoachingPersona;
use pierre_core::models::{User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::a2a::{auth::A2AClient, client::A2ASession, protocol::TaskStatus};
use tokio::time::sleep;
use uuid::Uuid;

async fn create_test_client(repos: &RepositoryRegistry) -> (A2AClient, Uuid) {
    let (client, user_id, _api_key_id) = create_test_client_with_window(repos, 3600).await;
    (client, user_id)
}

/// A user, an API key and an A2A client owned by that user, with the
/// client's rate-limit window set to `rate_limit_window_seconds`. Returns the
/// client as it was handed to the repository, the owner, and the API key id
/// the client was registered against.
async fn create_test_client_with_window(
    repos: &RepositoryRegistry,
    rate_limit_window_seconds: u32,
) -> (A2AClient, Uuid, String) {
    let unique_id = Uuid::new_v4();

    // First create a test user
    let test_user_id = Uuid::new_v4();
    let user = User {
        id: test_user_id,
        email: format!("test_{unique_id}@example.com"),
        display_name: Some(format!("Test User {unique_id}")),
        password_hash: format!("test_hash_{unique_id}"),
        tier: UserTier::Professional,
        strava_token: None,
        fitbit_token: None,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    repos
        .users
        .create(&user)
        .await
        .expect("Failed to create test user");

    // Create a test API key for the user
    let api_key = ApiKey {
        id: format!("test_api_key_{unique_id}"),
        user_id: test_user_id,
        name: format!("Test API Key {unique_id}"),
        description: Some("Test API key for A2A client".into()),
        key_prefix: format!("pk_test_{}", &unique_id.to_string()[0..8]),
        key_hash: format!("test_key_hash_{unique_id}"),
        tier: ApiKeyTier::Professional,
        rate_limit_requests: 1000,
        rate_limit_window_seconds: 3600,
        is_active: true,
        created_at: Utc::now(),
        last_used_at: None,
        expires_at: None,
    };
    repos
        .api_keys
        .create(&api_key)
        .await
        .expect("Failed to create test API key");

    let client = A2AClient {
        id: format!("test_client_{unique_id}"),
        name: format!("Test Client {unique_id}"),
        description: format!("Test A2A client {unique_id}"),
        public_key: format!("test_public_key_{unique_id}"),
        user_id: test_user_id,
        capabilities: vec!["fitness-data-analysis".into()],
        redirect_uris: vec!["https://test.example.com".into()],
        // The canonical a2a_clients schema dropped the permissions column; it is no
        // longer persisted and reads back as the default ["read_activities"].
        permissions: vec!["read_activities".into()],
        rate_limit_requests: 1000,
        rate_limit_window_seconds,
        is_active: true,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    repos
        .a2a
        .create_client(&client, "test_secret", &api_key.id)
        .await
        .expect("Failed to create A2A client");
    (client, test_user_id, api_key.id)
}

/// The client the repository hands back is the one it was given: the public
/// key the API serves, the rate limit and window the rate-limit route
/// reports, the capability and redirect lists; it is reachable through the
/// API key it was registered against, which is how API-key auth resolves a
/// caller; and the nil uuid lists every client, which is the admin listing.
#[tokio::test]
async fn test_a2a_client_round_trips_through_every_read() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, user_id, api_key_id) = create_test_client_with_window(&repos, 3600).await;

    let by_id = repos
        .a2a
        .get_client(&client.id)
        .await
        .expect("get_client must not error")
        .expect("client must exist by id");
    let by_api_key = repos
        .a2a
        .get_client_by_api_key_id(&api_key_id)
        .await
        .expect("get_client_by_api_key_id must not error")
        .expect("client must resolve through the API key it was registered against");
    let by_name = repos
        .a2a
        .get_client_by_name(&client.name)
        .await
        .expect("get_client_by_name must not error")
        .expect("client must exist by name");
    let listed = repos
        .a2a
        .list_clients(&user_id)
        .await
        .expect("list_clients must not error");
    let listed_for_owner = listed
        .iter()
        .find(|c| c.id == client.id)
        .expect("client must be listed for its owner");
    let listed_for_all = repos
        .a2a
        .list_clients(&Uuid::nil())
        .await
        .expect("list_clients(nil) must not error");
    let listed_for_admin = listed_for_all
        .iter()
        .find(|c| c.id == client.id)
        .expect("the nil uuid lists every client");

    for (read, got) in [
        ("get_client", &by_id),
        ("get_client_by_api_key_id", &by_api_key),
        ("get_client_by_name", &by_name),
        ("list_clients(owner)", listed_for_owner),
        ("list_clients(nil)", listed_for_admin),
    ] {
        assert_eq!(got.id, client.id, "{read}: id");
        assert_eq!(got.user_id, user_id, "{read}: user_id");
        assert_eq!(got.public_key, client.public_key, "{read}: public_key");
        assert_eq!(
            got.rate_limit_requests, client.rate_limit_requests,
            "{read}: rate_limit_requests"
        );
        assert_eq!(
            got.rate_limit_window_seconds, client.rate_limit_window_seconds,
            "{read}: rate_limit_window_seconds"
        );
        assert_eq!(
            got.capabilities, client.capabilities,
            "{read}: capabilities"
        );
        assert_eq!(
            got.redirect_uris, client.redirect_uris,
            "{read}: redirect_uris"
        );
        assert!(got.is_active, "{read}: is_active");
        assert_eq!(
            got.created_at.timestamp_millis(),
            client.created_at.timestamp_millis(),
            "{read}: created_at"
        );
    }

    let (id, secret_hash) = repos
        .a2a
        .get_client_credentials(&client.id)
        .await
        .expect("get_client_credentials must not error")
        .expect("credentials must exist");
    assert_eq!(id, client.id);
    assert_ne!(
        secret_hash, client.public_key,
        "the public key must never be the secret hash"
    );
    assert_eq!(
        secret_hash.len(),
        64,
        "the secret is stored as its SHA-256 hex digest"
    );
}

/// The current-usage count is bounded by the client's own rate-limit
/// window, not by a fixed hour: a request older than a 60-second window is
/// not current.
#[tokio::test]
async fn test_a2a_current_usage_honours_the_client_window() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id, _api_key_id) = create_test_client_with_window(&repos, 60).await;

    for age in [chrono::Duration::minutes(5), chrono::Duration::seconds(5)] {
        let usage = A2AUsage {
            id: None,
            client_id: client.id.clone(),
            session_token: None,
            timestamp: Utc::now() - age,
            tool_name: "analyze".into(),
            request_size_bytes: None,
            response_size_bytes: None,
            response_time_ms: Some(10),
            status_code: 200,
            error_message: None,
            ip_address: None,
            user_agent: None,
            protocol_version: "1.0".into(),
            client_capabilities: vec![],
            granted_scopes: vec![],
        };
        repos
            .a2a
            .record_usage(&usage)
            .await
            .expect("record_usage must succeed");
    }

    let current = repos
        .a2a
        .get_client_current_usage(&client.id)
        .await
        .expect("get_client_current_usage must not error");
    assert_eq!(
        current, 1,
        "only the request inside the client's 60 s window is current"
    );

    let missing = repos
        .a2a
        .get_client_current_usage("no-such-client")
        .await
        .expect_err("an unknown client has no window to count in");
    assert!(
        missing.to_string().contains("no-such-client"),
        "the error names the client: {missing}"
    );
}

#[tokio::test]
async fn test_a2a_client_management() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, user_id) = create_test_client(&repos).await;

    // Get client
    let retrieved = repos
        .a2a
        .get_client(&client.id)
        .await
        .expect("Failed to get A2A client")
        .expect("Client not found");

    assert_eq!(retrieved.id, client.id);
    assert_eq!(retrieved.name, client.name);
    assert_eq!(retrieved.permissions, client.permissions);

    // List clients - check that our client is in the list
    let clients = repos
        .a2a
        .list_clients(&user_id)
        .await
        .expect("Failed to list A2A clients");

    // Find our client in the list
    let found_client = clients.iter().find(|c| c.id == client.id);
    assert!(
        found_client.is_some(),
        "Created client should be in the list"
    );
    assert_eq!(found_client.unwrap().id, client.id);
}

#[tokio::test]
async fn test_a2a_session_management() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;

    // Create session (without user_id to avoid foreign key constraint)
    let session = A2ASession {
        id: format!("session_{}", Uuid::new_v4()),
        client_id: client.id.clone(),
        user_id: None, // No user association for this test
        granted_scopes: vec!["read".into(), "write".into()],
        expires_at: Utc::now() + chrono::Duration::hours(1),
        last_activity: Utc::now(),
        created_at: Utc::now(),
        requests_count: 0,
    };

    let session_token = repos
        .a2a
        .create_session(
            &session.client_id,
            session.user_id.as_ref(),
            &session.granted_scopes,
            1,
        )
        .await
        .expect("Failed to create A2A session");

    // Get session
    let retrieved = repos
        .a2a
        .get_session(&session_token)
        .await
        .expect("Failed to get A2A session")
        .expect("Session not found");

    assert_eq!(retrieved.id, session_token);
    assert_eq!(retrieved.client_id, session.client_id);
    assert_eq!(retrieved.granted_scopes, session.granted_scopes);

    // Update session activity
    repos
        .a2a
        .update_session_activity(&session_token)
        .await
        .expect("Failed to update session activity");

    // Test getting active sessions for client
    let active_sessions = repos
        .a2a
        .get_active_sessions(&client.id)
        .await
        .expect("Failed to get active sessions");

    assert_eq!(active_sessions.len(), 1);
    assert_eq!(active_sessions[0].id, session_token);
    assert_eq!(active_sessions[0].client_id, client.id);
}

#[tokio::test]
async fn test_a2a_task_management() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;

    let session_token = repos
        .a2a
        .create_session(&client.id, None, &["read".into()], 1)
        .await
        .expect("Failed to create A2A session");

    // Create task — new tasks start in the A2A 1.0 `submitted` state and
    // carry the server-assigned contextId.
    let input_data = serde_json::json!({"data": "test"});
    let task_id = repos
        .a2a
        .create_task(
            &client.id,
            Some(&session_token),
            "analysis",
            &input_data,
            Some("ctx-test-1"),
        )
        .await
        .expect("Failed to create A2A task");

    // Get task
    let retrieved = repos
        .a2a
        .get_task(&task_id)
        .await
        .expect("Failed to get A2A task")
        .expect("Task not found");

    assert_eq!(retrieved.id, task_id);
    assert_eq!(retrieved.status, TaskStatus::Submitted);
    assert_eq!(retrieved.context_id.as_deref(), Some("ctx-test-1"));
    assert_eq!(retrieved.input_data, input_data);

    // Persist wire history/artifacts JSON.
    let history = serde_json::json!([{
        "messageId": "m-1",
        "role": "ROLE_USER",
        "parts": [{ "text": "run analysis" }]
    }]);
    let artifacts = serde_json::json!([{
        "artifactId": "a-1",
        "parts": [{ "data": { "ok": true } }]
    }]);
    repos
        .a2a
        .update_task_wire_state(&task_id, Some(&history), Some(&artifacts))
        .await
        .expect("Failed to update task wire state");

    // Update task status with a result and a status message.
    let result = serde_json::json!({"result": "success"});
    let status_message = serde_json::json!({
        "messageId": "m-2",
        "role": "ROLE_AGENT",
        "parts": [{ "text": "done" }]
    });
    repos
        .a2a
        .update_task_status(
            &task_id,
            &TaskStatus::Completed,
            Some(&result),
            Some(&status_message),
        )
        .await
        .expect("Failed to update task status");

    // Verify update
    let updated = repos
        .a2a
        .get_task(&task_id)
        .await
        .expect("Failed to get updated task")
        .expect("Task not found");

    assert_eq!(updated.status, TaskStatus::Completed);
    assert!(updated.status.is_terminal());
    assert_eq!(updated.result, Some(result));
    assert_eq!(updated.status_message, Some(status_message));
    assert_eq!(updated.history, Some(history));
    assert_eq!(updated.artifacts, Some(artifacts));

    // ListTasks filters: context + status, ordered by status timestamp.
    let listed = repos
        .a2a
        .list_tasks(
            Some(&session_token),
            Some(&TaskStatus::Completed),
            Some("ctx-test-1"),
            None,
            Some(10),
            None,
        )
        .await
        .expect("Failed to list A2A tasks");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, task_id);

    let none_listed = repos
        .a2a
        .list_tasks(
            Some(&session_token),
            Some(&TaskStatus::Working),
            None,
            None,
            Some(10),
            None,
        )
        .await
        .expect("Failed to list A2A tasks");
    assert!(none_listed.is_empty());
}

#[tokio::test]
async fn test_a2a_push_notification_config_crud() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;
    let session_token = repos
        .a2a
        .create_session(&client.id, None, &["read".into()], 1)
        .await
        .expect("Failed to create A2A session");
    let task_id = repos
        .a2a
        .create_task(
            &client.id,
            Some(&session_token),
            "message",
            &serde_json::json!({}),
            None,
        )
        .await
        .expect("Failed to create A2A task");

    let config = A2APushNotificationConfig {
        config_id: "cfg-1".into(),
        task_id: task_id.clone(),
        url: "https://webhooks.example.com/a2a".into(),
        token: Some("client-token".into()),
        auth_scheme: Some("Bearer".into()),
        auth_credentials: Some("secret".into()),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    repos
        .a2a
        .create_push_config(&config)
        .await
        .expect("Failed to create push config");

    let fetched = repos
        .a2a
        .get_push_config(&task_id, "cfg-1")
        .await
        .expect("Failed to get push config")
        .expect("Push config not found");
    assert_eq!(fetched.url, config.url);
    assert_eq!(fetched.token, config.token);
    assert_eq!(fetched.auth_scheme, config.auth_scheme);

    // Upsert: same (task_id, config_id) replaces the registration.
    let mut updated = config.clone();
    updated.url = "https://webhooks.example.com/a2a/v2".into();
    repos
        .a2a
        .create_push_config(&updated)
        .await
        .expect("Failed to upsert push config");

    let listed = repos
        .a2a
        .list_push_configs(&task_id)
        .await
        .expect("Failed to list push configs");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].url, updated.url);

    // Delete reports whether a row existed.
    assert!(repos
        .a2a
        .delete_push_config(&task_id, "cfg-1")
        .await
        .expect("Failed to delete push config"));
    assert!(!repos
        .a2a
        .delete_push_config(&task_id, "cfg-1")
        .await
        .expect("Failed to delete push config twice"));
    assert!(repos
        .a2a
        .get_push_config(&task_id, "cfg-1")
        .await
        .expect("Failed to get push config")
        .is_none());
}

#[tokio::test]
async fn test_a2a_usage_tracking() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;

    // Record usage
    let usage = A2AUsage {
        id: None,
        client_id: client.id.clone(),
        session_token: None, // No session for this test
        timestamp: Utc::now(),
        tool_name: "analyze".into(),
        request_size_bytes: Some(256),
        response_size_bytes: Some(512),
        response_time_ms: Some(100),
        status_code: 200,
        error_message: None,
        ip_address: Some("127.0.0.1".to_owned()),
        user_agent: Some("test-agent".into()),
        protocol_version: "1.0".into(),
        client_capabilities: vec!["analysis".into()],
        granted_scopes: vec!["read".into()],
    };

    repos
        .a2a
        .record_usage(&usage)
        .await
        .expect("Failed to record A2A usage");

    // Check current usage
    let current_usage = repos
        .a2a
        .get_client_current_usage(&client.id)
        .await
        .expect("Failed to get current usage");
    assert_eq!(current_usage, 1);

    // Get usage stats
    let stats = repos
        .a2a
        .get_usage_stats(
            &client.id,
            Utc::now() - chrono::Duration::hours(1),
            Utc::now() + chrono::Duration::hours(1),
        )
        .await
        .expect("Failed to get usage stats");

    assert_eq!(stats.total_requests, 1);
    assert_eq!(stats.successful_requests, 1);
    assert_eq!(stats.failed_requests, 0);
}

#[tokio::test]
async fn test_a2a_schema_no_duplicate_columns() {
    let db = create_test_db().await.expect("Failed to create database");

    // Each backend has its own catalogue; both list the migrated columns.
    let columns: Vec<(String,)> = match &db {
        Database::SQLite(sqlite) => {
            sqlx::query_as("SELECT name FROM pragma_table_info('a2a_clients') ORDER BY name")
                .fetch_all(sqlite.pool())
                .await
                .expect("Failed to query table info")
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(pg) => sqlx::query_as(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_schema = current_schema() AND table_name = 'a2a_clients' \
             ORDER BY column_name",
        )
        .fetch_all(pg.pool())
        .await
        .expect("Failed to query table info"),
    };

    let column_names: Vec<String> = columns.into_iter().map(|(name,)| name).collect();

    assert!(
        column_names.contains(&"capabilities".to_owned()),
        "capabilities column should exist in a2a_clients table"
    );
    assert!(
        column_names.contains(&"redirect_uris".to_owned()),
        "redirect_uris column should exist in a2a_clients table"
    );

    let capabilities_count = column_names.iter().filter(|n| *n == "capabilities").count();
    let redirect_uris_count = column_names
        .iter()
        .filter(|n| *n == "redirect_uris")
        .count();

    assert_eq!(
        capabilities_count, 1,
        "capabilities column should appear exactly once (found {capabilities_count} occurrences)"
    );
    assert_eq!(
        redirect_uris_count, 1,
        "redirect_uris column should appear exactly once (found {redirect_uris_count} occurrences)"
    );
}

/// A session past its expiry is not live, whatever the clock's text form:
/// the expiry is compared as an instant, not as two strings that happen to
/// spell the same day differently.
#[tokio::test]
async fn test_a2a_expired_session_is_not_live() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;

    let expired = repos
        .a2a
        .create_session(&client.id, None, &["read".to_owned()], -1)
        .await
        .expect("create_session must succeed");
    let live = repos
        .a2a
        .create_session(&client.id, None, &["read".to_owned()], 1)
        .await
        .expect("create_session must succeed");

    assert!(
        repos
            .a2a
            .get_session(&expired)
            .await
            .expect("get_session must not error")
            .is_none(),
        "a session that expired an hour ago is not returned"
    );
    let active = repos
        .a2a
        .get_active_sessions(&client.id)
        .await
        .expect("get_active_sessions must not error");
    assert_eq!(
        active.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec![live.as_str()],
        "only the live session is active"
    );

    repos
        .a2a
        .invalidate_client_sessions(&client.id)
        .await
        .expect("invalidate_client_sessions must succeed");
    assert!(
        repos
            .a2a
            .get_session(&live)
            .await
            .expect("get_session must not error")
            .is_none(),
        "an invalidated session is not live either"
    );
}

/// `updated_after` sees a task whose status moved after the cutoff, and the
/// listing puts it first: a status update stamps `updated_at` in the same
/// form every other write and read uses.
#[tokio::test]
async fn test_a2a_list_tasks_updated_after_sees_the_updated_task() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;

    let untouched = repos
        .a2a
        .create_task(&client.id, None, "analyze", &serde_json::json!({}), None)
        .await
        .expect("create_task must succeed");
    let moved = repos
        .a2a
        .create_task(&client.id, None, "analyze", &serde_json::json!({}), None)
        .await
        .expect("create_task must succeed");
    let cutoff = repos
        .a2a
        .get_task(&moved)
        .await
        .expect("get_task must not error")
        .expect("task must exist")
        .updated_at;

    sleep(Duration::from_millis(20)).await;
    repos
        .a2a
        .update_task_status(&moved, &TaskStatus::Working, None, None)
        .await
        .expect("update_task_status must succeed");

    let after = repos
        .a2a
        .list_tasks(Some(&client.id), None, None, Some(cutoff), None, None)
        .await
        .expect("list_tasks must not error");
    assert_eq!(
        after.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![moved.as_str()],
        "the task updated after the cutoff is the one returned"
    );

    let all = repos
        .a2a
        .list_tasks(Some(&client.id), None, None, None, None, None)
        .await
        .expect("list_tasks must not error");
    assert_eq!(
        all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        vec![moved.as_str(), untouched.as_str()],
        "most recently updated first"
    );
    let moved_task = repos
        .a2a
        .get_task(&moved)
        .await
        .expect("get_task must not error")
        .expect("task must exist");
    assert!(
        moved_task.updated_at > cutoff,
        "the status update advanced updated_at past its creation stamp"
    );
    assert_eq!(moved_task.status, TaskStatus::Working);
}

fn usage_at(client_id: &str, at: chrono::DateTime<Utc>, status_code: u16) -> A2AUsage {
    A2AUsage {
        id: None,
        client_id: client_id.to_owned(),
        session_token: None,
        timestamp: at,
        tool_name: "analyze".into(),
        request_size_bytes: Some(256),
        response_size_bytes: Some(512),
        response_time_ms: Some(100),
        status_code,
        error_message: None,
        ip_address: None,
        user_agent: None,
        protocol_version: "1.0".into(),
        client_capabilities: vec!["analysis".into()],
        granted_scopes: vec!["read".into()],
    }
}

/// Usage history is one row per UTC calendar day, newest day first — the
/// caller (`ClientManager::get_client_usage`) reads the first row as the
/// last request — with a 2xx/3xx counted as a success and a 4xx/5xx as an
/// error.
#[tokio::test]
async fn test_a2a_usage_history_is_per_day_newest_first() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;
    let now = Utc::now();
    let yesterday = now - chrono::Duration::days(1);
    for (at, code) in [(yesterday, 200), (now, 200), (now, 302), (now, 500)] {
        repos
            .a2a
            .record_usage(&usage_at(&client.id, at, code))
            .await
            .expect("record_usage must succeed");
    }

    let history = repos
        .a2a
        .get_client_usage_history(&client.id, 7)
        .await
        .expect("get_client_usage_history must not error");
    let days: Vec<(chrono::NaiveDate, u32, u32)> = history
        .iter()
        .map(|(day, ok, err)| (day.date_naive(), *ok, *err))
        .collect();
    assert_eq!(
        days,
        vec![(now.date_naive(), 2, 1), (yesterday.date_naive(), 1, 0)],
        "two calendar days, today first, with today's 200 and 302 as successes and its 500 as an error"
    );
    assert_eq!(
        history[0].0.time(),
        chrono::NaiveTime::MIN,
        "a day is reported at its midnight"
    );
}

/// Usage stats partition every request into successful (< 400) and failed
/// (>= 400), so the two sum to the total; a redirect is not an error.
#[tokio::test]
async fn test_a2a_usage_stats_partition_on_the_400_boundary() {
    let db = create_test_db()
        .await
        .expect("Failed to create test database");
    let repos = db.repositories();

    let (client, _user_id) = create_test_client(&repos).await;
    let now = Utc::now();
    for code in [200, 302, 404, 500] {
        repos
            .a2a
            .record_usage(&usage_at(&client.id, now, code))
            .await
            .expect("record_usage must succeed");
    }

    let stats = repos
        .a2a
        .get_usage_stats(
            &client.id,
            now - chrono::Duration::hours(1),
            now + chrono::Duration::hours(1),
        )
        .await
        .expect("get_usage_stats must not error");
    assert_eq!(stats.total_requests, 4);
    assert_eq!(stats.successful_requests, 2, "200 and 302 succeeded");
    assert_eq!(stats.failed_requests, 2, "404 and 500 failed");
    assert_eq!(
        stats.successful_requests + stats.failed_requests,
        stats.total_requests,
        "every request is counted exactly once"
    );
    assert_eq!(stats.avg_response_time_ms, Some(100));
    assert_eq!(stats.total_request_bytes, Some(4 * 256));
    assert_eq!(stats.total_response_bytes, Some(4 * 512));
}
