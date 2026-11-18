// ABOUTME: Response date goldens for an account's own routes: API keys and their usage window, MCP tokens, and the data export
// ABOUTME: Pins key stamps written with Z, token stamps with an offset, the usage window as read, and the export's rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/keys`, `/api/keys/{id}/usage`,
//! `/api/user/mcp-tokens`, `/api/user/export` (carnet#1620, gate 3).
//!
//! An API key is stored with the stamps the suite chooses and read back
//! through the server's whole HTTP app. What a route creates is stamped with
//! the wall clock and compared by shape, which keeps the clock's fraction:
//!
//! - a created key's `created_at` and `expires_at`;
//! - a created MCP token's `created_at` and `expires_at`;
//! - the export's `generated_at`, the day in its file name, and the
//!   account's `last_active`, which signing in moves with the database's clock
//!   (compared by its whole shape: whole seconds on SQLite).
//!
//! The MCP token's row and the membership row are then moved to chosen
//! instants, and the token list and the export's `invited_at` and
//! `joined_at` are compared as literals.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;
use std::time::Duration as Wait;

use axum::http::StatusCode;
use axum::Router;
use pierre_auth::api_keys::ApiKeyTier;
use pierre_core::models::ApiKey;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use tokio::time;

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::date_golden::{
    as_stored, dates_in, dates_in_stamped, get, instant, on_database_clock, on_the_clock, restamp,
    seed_account, send, shape, table, with_json, Athlete,
};

struct Account {
    resources: Arc<ServerContext>,
    app: Router,
    athlete: Athlete,
}

impl Account {
    /// An account created at 10:00:00 on 2026-08-02, signed in.
    async fn new(label: &str) -> Self {
        let resources = common::create_test_server_resources().await.unwrap();
        let app = ProviderToolRouter::build_http_app(&resources);
        let athlete = seed_account(
            &resources,
            label,
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            instant("2026-08-02T10:00:00"),
        )
        .await;
        Self {
            resources,
            app,
            athlete,
        }
    }

    async fn read(&self, uri: &str) -> (StatusCode, Value) {
        send(self.app.clone(), get(uri, &self.athlete.token)).await
    }

    async fn create(&self, uri: &str, body: &Value) -> Value {
        let (status, answered) = send(
            self.app.clone(),
            with_json("POST", uri, &self.athlete.token, body),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{uri}: {answered}");
        answered
    }

    /// A key stored with the stamps the suite chooses.
    async fn stored_key(&self) -> String {
        let id = format!("golden-key-{}", self.athlete.user_id.simple());
        self.resources
            .common
            .repos
            .api_keys
            .create(&ApiKey {
                id: id.clone(),
                user_id: self.athlete.user_id,
                name: "Stored key".to_owned(),
                key_prefix: format!("gld_{}_", self.athlete.user_id.simple()),
                key_hash: format!("hash-{}", self.athlete.user_id.simple()),
                description: None,
                tier: ApiKeyTier::Starter,
                rate_limit_requests: 10,
                rate_limit_window_seconds: 3_600,
                is_active: true,
                last_used_at: Some(instant("2026-08-29T10:15:30.250125")),
                expires_at: Some(instant("2099-12-31T23:59:59")),
                created_at: instant("2026-08-28T02:59:03.500250"),
            })
            .await
            .unwrap();
        id
    }
}

/// The two bounds a read echoes, when it is not refused.
type Echoed = Option<(&'static str, &'static str)>;

const MOVE_TOKEN: &str = "UPDATE user_mcp_tokens SET created_at = $1 WHERE CAST(id AS TEXT) = $2";
const EXPIRE_TOKEN: &str = "UPDATE user_mcp_tokens SET expires_at = $1 WHERE CAST(id AS TEXT) = $2";
const USE_TOKEN: &str = "UPDATE user_mcp_tokens SET last_used_at = $1 WHERE CAST(id AS TEXT) = $2";
const INVITE_MEMBER: &str = "UPDATE tenant_users SET invited_at = $1 \
                             WHERE CAST(user_id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const JOIN_MEMBER: &str = "UPDATE tenant_users SET joined_at = $1 \
                           WHERE CAST(user_id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";

#[tokio::test]
async fn a_stored_key_is_listed_with_its_stamps_in_utc_with_z() {
    let account = Account::new("golden-keys-list").await;
    let id = account.stored_key().await;

    let (status, body) = account.read("/api/keys").await;

    assert_eq!(status, StatusCode::OK, "{body}");
    let key = body["api_keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["id"] == id.as_str())
        .unwrap_or_else(|| panic!("the stored key is listed: {body}"));
    assert_eq!(
        dates_in(key),
        table(&[
            ("/created_at", "2026-08-28T02:59:03.500250Z"),
            ("/expires_at", "2099-12-31T23:59:59Z"),
            ("/last_used_at", "2026-08-29T10:15:30.250125Z"),
        ]),
        "{key}"
    );
}

#[tokio::test]
async fn a_created_key_is_stamped_with_z() {
    on_the_clock(|| async {
        let account = Account::new("golden-keys-create").await;

        let created = account
            .create(
                "/api/keys",
                &json!({ "name": "Thirty days", "description": "golden", "expires_in_days": 30 }),
            )
            .await;

        assert_eq!(
            dates_in_stamped(&created["key_info"], &["/created_at", "/expires_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.fZ"),
                ("/expires_at", "dddd-dd-ddTdd:dd:dd.fZ"),
            ]),
            "{created}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_usage_window_is_read_as_two_instants_and_echoed_in_utc_with_z() {
    let account = Account::new("golden-keys-usage").await;
    let id = account.stored_key().await;

    // (start_date, end_date, status, the window echoed when it is read).
    let read: [(&str, &str, u16, Echoed); 6] = [
        (
            "2026-08-01T00:00:00Z",
            "2026-08-31T23:59:59Z",
            200,
            Some(("2026-08-01T00:00:00Z", "2026-08-31T23:59:59Z")),
        ),
        // An offset and a fraction are read, and echoed in UTC.
        (
            "2026-08-01T05:30:00+05:30",
            "2026-08-31T19:59:59.500-04:00",
            200,
            Some(("2026-08-01T00:00:00Z", "2026-08-31T23:59:59.500Z")),
        ),
        // A space for the `T` is read.
        (
            "2026-08-01 00:00:00Z",
            "2026-08-31 23:59:59Z",
            200,
            Some(("2026-08-01T00:00:00Z", "2026-08-31T23:59:59Z")),
        ),
        // Not instants: a date alone, no zone, nothing.
        ("2026-08-01", "2026-08-31", 400, None),
        ("2026-08-01T00:00:00", "2026-08-31T23:59:59", 400, None),
        ("", "", 400, None),
    ];
    for (start, end, status, echoed) in read {
        let uri = format!(
            "/api/keys/{id}/usage?start_date={}&end_date={}",
            urlencoding::encode(start),
            urlencoding::encode(end)
        );
        let (answered, body) = account.read(&uri).await;
        assert_eq!(answered.as_u16(), status, "{uri}: {body}");
        if let Some((start, end)) = echoed {
            assert_eq!(
                dates_in(&body),
                table(&[("/stats/period_end", end), ("/stats/period_start", start)]),
                "{uri}: {body}"
            );
        }
    }
}

#[tokio::test]
async fn an_mcp_token_is_stamped_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let account = Account::new("golden-mcp-token").await;

        let created = account
            .create(
                "/api/user/mcp-tokens",
                &json!({ "name": "Thirty days", "expires_in_days": 30 }),
            )
            .await;
        assert_eq!(
            dates_in_stamped(&created, &["/created_at", "/expires_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/expires_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{created}"
        );

        // The list answers the same two stamps as the database keeps them:
        // character for character on SQLite, cut to the microsecond on
        // PostgreSQL.
        let (status, listed) = account.read("/api/user/mcp-tokens").await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let token = &listed["tokens"][0];
        let database = &account.resources.agent.database;
        let kept = |field: &str| as_stored(database, created[field].as_str().unwrap());
        assert_eq!(
            (
                token["created_at"].as_str().unwrap(),
                token["expires_at"].as_str().unwrap()
            ),
            (kept("created_at").as_str(), kept("expires_at").as_str()),
            "{listed}"
        );
        assert!(token["last_used_at"].is_null(), "{listed}");

        // Moved to chosen instants, the row answers them with an offset and the
        // fraction it holds.
        let id = created["id"].as_str().unwrap();
        for (sql, at) in [
            (MOVE_TOKEN, "2026-08-28T02:59:03.500250"),
            (EXPIRE_TOKEN, "2026-09-27T02:59:03"),
            (USE_TOKEN, "2026-08-29T10:15:30.250125"),
        ] {
            restamp(database, sql, instant(at), id, None).await;
        }
        let (status, listed) = account.read("/api/user/mcp-tokens").await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(
            dates_in(&listed),
            table(&[
                ("/tokens/0/created_at", "2026-08-28T02:59:03.500250+00:00"),
                ("/tokens/0/expires_at", "2026-09-27T02:59:03+00:00"),
                ("/tokens/0/last_used_at", "2026-08-29T10:15:30.250125+00:00"),
            ]),
            "{listed}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_export_answers_the_accounts_rows_as_the_database_holds_them() {
    on_the_clock(|| async {
        let account = Account::new("golden-export").await;
        let auth = format!("Bearer {}", account.athlete.token);
        let database = &account.resources.agent.database;
        let user = account.athlete.user_id.to_string();
        restamp(
            database,
            INVITE_MEMBER,
            instant("2026-08-02T10:00:00.250125"),
            &user,
            Some(account.athlete.tenant),
        )
        .await;
        restamp(
            database,
            JOIN_MEMBER,
            instant("2026-08-03T11:30:05.250125"),
            &user,
            Some(account.athlete.tenant),
        )
        .await;

        // The server writes four exports at once, process-wide, and the suites
        // run theirs in parallel: a busy refusal is retried, as it asks.
        let mut response = None;
        for _ in 0..200 {
            let answered = AxumTestRequest::get("/api/user/export")
                .header("Authorization", &auth)
                .send(account.app.clone())
                .await;
            if !answered.body_text().contains("\"export_busy\"") {
                response = Some(answered);
                break;
            }
            time::sleep(Wait::from_millis(50)).await;
        }
        let response = response.expect("the export stopped being busy");
        assert_eq!(response.status(), 200, "{}", response.body_text());

        // The file is named after today.
        let file = response.header("content-disposition").unwrap().to_owned();
        assert_eq!(
            shape(&file),
            "dddd-dd-dd",
            "the name carries one calendar date: {file}"
        );
        assert!(
            file.starts_with("attachment; filename=\"dravr-export-") && file.ends_with(".json\""),
            "{file}"
        );

        let doc: Value = serde_json::from_str(&response.body_text()).unwrap();
        assert_eq!(doc["complete"], true, "{}", doc["failed_tables"]);

        // Signing in moved `last_active`, with the database's own clock: SQLite
        // writes it with a space and no zone in whole seconds, PostgreSQL as an
        // instant to the microsecond (run on CI's PostgreSQL lane).
        let last_active = match account.resources.agent.database.backend() {
            DatabaseBackend::SQLite(_) => "dddd-dd-dd dd:dd:dd",
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(_) => "dddd-dd-ddTdd:dd:dd.f+dd:dd",
        };
        assert_eq!(
            dates_in_stamped(
                &json!({ "generated_at": doc["generated_at"] }),
                &["/generated_at"]
            ),
            table(&[("/generated_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "generated_at is a reading of now with an offset: {}",
            doc["generated_at"]
        );
        assert_eq!(
            on_database_clock(
                &account.resources.agent.database,
                dates_in(&doc["tables"]["users"]),
                &["/0/last_active"]
            ),
            table(&[
                ("/0/approved_at", "2026-08-02T10:00:00+00:00"),
                ("/0/created_at", "2026-08-02T10:00:00+00:00"),
                ("/0/last_active", last_active),
            ]),
            "{}",
            doc["tables"]["users"]
        );
        assert_eq!(
            dates_in(&doc["tables"]["tenant_users"]),
            table(&[
                ("/0/invited_at", "2026-08-02T10:00:00.250125+00:00"),
                ("/0/joined_at", "2026-08-03T11:30:05.250125+00:00"),
            ]),
            "{}",
            doc["tables"]["tenant_users"]
        );
    })
    .await;
}
