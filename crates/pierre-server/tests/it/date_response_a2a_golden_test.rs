// ABOUTME: Response date goldens for A2A tasks over JSON-RPC: the status timestamp of a task and the instant ListTasks filters on
// ABOUTME: Pins milliseconds with Z whatever the row holds, and which shapes statusTimestampAfter reads or refuses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `GetTask` and `ListTasks` on `/a2a/jsonrpc`
//! (carnet#1620, gate 3).
//!
//! A task's status timestamp is its row's `updated_at`, written by hand at
//! exactly three fraction digits with `Z`. Tasks are opened through the
//! repository, which stamps them with the wall clock, then moved to instants
//! the suite chooses with one `UPDATE` each; every read goes through the
//! server's whole HTTP app with a delegated grant, as an A2A client calls.
//!
//! `statusTimestampAfter` is read by hand as an RFC 3339 instant and compared
//! with the stored instant, fraction included.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::Router;
use pierre_a2a::events::TASK_EVENTS;
use pierre_a2a::ClientRegistrationRequest;
use pierre_auth::auth::DelegatedGrant;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::date_golden::{
    dates_in, instant, lies_between, restamp, seed_account, send, shape, table, with_json, Instant,
};

const MOVE_TASK: &str = "UPDATE a2a_tasks SET updated_at = $1 WHERE task_id = $2";

struct Caller {
    resources: Arc<ServerContext>,
    app: Router,
    client_id: String,
    grant: String,
}

impl Caller {
    async fn new(label: &str) -> Self {
        let resources = common::create_test_server_resources().await.unwrap();
        let app = ProviderToolRouter::build_http_app(&resources);
        let owner = seed_account(
            &resources,
            label,
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            instant("2026-08-02T10:00:00"),
        )
        .await;
        let client_id = resources
            .a2a
            .a2a_client_manager
            .register_client(
                ClientRegistrationRequest {
                    name: format!("{label}-{}", owner.user_id),
                    description: "date golden client".to_owned(),
                    capabilities: vec!["fitness-data-analysis".to_owned()],
                    redirect_uris: Vec::new(),
                    contact_email: "golden@example.com".to_owned(),
                },
                owner.user_id,
            )
            .await
            .unwrap()
            .client_id;
        let scopes = ["fitness:read".to_owned(), "profile:read".to_owned()];
        let grant = resources
            .auth
            .auth_manager
            .generate_oauth_access_token(
                &resources.auth.jwks_manager,
                &DelegatedGrant {
                    client_id: "copilot_studio_test",
                    scopes: &scopes,
                    audience: None,
                },
                &owner.user_id,
                Some(owner.tenant.to_string()),
            )
            .unwrap();
        Self {
            resources,
            app,
            client_id,
            grant,
        }
    }

    /// A task in `context`, last moved at `at` when one is given.
    async fn task(&self, context: &str, at: Option<&str>) -> String {
        let id = self
            .resources
            .common
            .repos
            .a2a
            .create_task(&self.client_id, None, "message", &json!({}), Some(context))
            .await
            .unwrap();
        if let Some(at) = at {
            restamp(
                &self.resources.agent.database,
                MOVE_TASK,
                instant(at),
                &id,
                None,
            )
            .await;
        }
        id
    }

    async fn rpc(&self, method: &str, params: Value) -> Value {
        let mut request = with_json(
            "POST",
            "/a2a/jsonrpc",
            &self.grant,
            &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        );
        let headers = request.headers_mut();
        headers.insert("accept", "application/json".parse().unwrap());
        headers.insert("A2A-Version", "1.0".parse().unwrap());
        send(self.app.clone(), request).await.1
    }
}

#[tokio::test]
async fn a_tasks_status_timestamp_is_written_at_the_millisecond_with_z() {
    let client = Caller::new("golden-a2a-get").await;

    // (the instant the row holds, the timestamp answered): three digits
    // always, the rest cut, never rounded.
    let stored = [
        ("2026-08-28T02:59:03.500", "2026-08-28T02:59:03.500Z"),
        ("2026-08-28T02:59:03", "2026-08-28T02:59:03.000Z"),
        ("2026-08-29T10:15:30.250125", "2026-08-29T10:15:30.250Z"),
        ("2026-08-29T23:59:59.999999", "2026-08-29T23:59:59.999Z"),
    ];
    for (at, answered) in stored {
        let id = client.task("ctx-get", Some(at)).await;
        let got = client.rpc("GetTask", json!({ "id": id })).await;
        assert_eq!(
            dates_in(&got),
            table(&[("/result/status/timestamp", answered)]),
            "{at}: {got}"
        );
    }

    // A task nobody moved carries the clock, at the same width.
    let id = client.task("ctx-get", None).await;
    let got = client.rpc("GetTask", json!({ "id": id })).await;
    let stamp = got["result"]["status"]["timestamp"].as_str().unwrap();
    assert_eq!(shape(stamp), "dddd-dd-ddTdd:dd:dd.dddZ", "{got}");
}

#[tokio::test]
async fn list_tasks_reads_its_instant_in_any_offset_and_lists_what_moved_after_it() {
    let client = Caller::new("golden-a2a-list").await;
    client.task("older", Some("2026-08-28T02:59:03.500")).await;
    client.task("newer", Some("2026-08-29T10:15:30")).await;

    let listed = client.rpc("ListTasks", json!({})).await;
    assert_eq!(
        dates_in(&listed),
        table(&[
            (
                "/result/tasks/0/status/timestamp",
                "2026-08-29T10:15:30.000Z"
            ),
            (
                "/result/tasks/1/status/timestamp",
                "2026-08-28T02:59:03.500Z"
            ),
        ]),
        "{listed}"
    );

    // (statusTimestampAfter, the contexts listed, or none when refused).
    let read: [(&str, Option<&[&str]>); 11] = [
        // The older task's own instant, as `Z`, in Kolkata and in Toronto: a
        // task that moved at the instant itself is not after it.
        ("2026-08-28T02:59:03.500Z", Some(&["newer"])),
        ("2026-08-28T08:29:03.500+05:30", Some(&["newer"])),
        ("2026-08-27T22:59:03.500-04:00", Some(&["newer"])),
        // One millisecond before, and the whole second.
        ("2026-08-28T02:59:03.499Z", Some(&["newer", "older"])),
        ("2026-08-28T02:59:03Z", Some(&["newer", "older"])),
        ("2026-08-29T10:15:30Z", Some(&[])),
        // A space for the `T`, and a lower-case `z`.
        ("2026-08-28 02:59:03.500Z", Some(&["newer"])),
        ("2026-08-28t02:59:03.500z", Some(&["newer"])),
        // Not instants: a date alone, no zone, an offset without its colon.
        ("2026-08-28", None),
        ("2026-08-28T02:59:03.500", None),
        ("2026-08-28T02:59:03.500+0000", None),
    ];
    let mut answered = Vec::new();
    for (after, _) in read {
        let listed = client
            .rpc("ListTasks", json!({ "statusTimestampAfter": after }))
            .await;
        let contexts: Option<Vec<String>> = listed["result"]["tasks"].as_array().map(|tasks| {
            tasks
                .iter()
                .map(|task| task["contextId"].as_str().unwrap().to_owned())
                .collect()
        });
        if contexts.is_none() {
            assert_eq!(
                listed["error"]["message"], "Invalid statusTimestampAfter: expected ISO-8601",
                "{after}: {listed}"
            );
        }
        answered.push((after, contexts));
    }
    let expected: Vec<(&str, Option<Vec<String>>)> = read
        .iter()
        .map(|(after, contexts)| {
            (
                *after,
                contexts.map(|names| names.iter().map(|name| (*name).to_owned()).collect()),
            )
        })
        .collect();
    assert_eq!(
        answered, expected,
        "the contexts each statusTimestampAfter lists, or its refusal"
    );
}

/// A cancelled task publishes a status-update event on the stream bus,
/// stamped with the wall clock as it is published: three digits and `Z`,
/// between two readings of the clock taken around the request. The
/// answer's own status is the row's, at the same width.
#[tokio::test]
async fn a_cancelled_tasks_streamed_status_is_stamped_at_the_millisecond_with_z() {
    let client = Caller::new("golden-a2a-cancel").await;
    let id = client.task("ctx-cancel", None).await;
    let mut stream = TASK_EVENTS.subscribe(&id);

    let before = Instant::now();
    let cancelled = client.rpc("CancelTask", json!({ "id": id })).await;
    let after = Instant::now();
    assert!(cancelled["error"].is_null(), "{cancelled}");

    let event = serde_json::to_value(stream.try_recv().unwrap()).unwrap();
    let rows = dates_in(&event);
    assert_eq!(rows.len(), 1, "{event}");
    let (pointer, stamp) = &rows[0];
    assert_eq!(pointer, "/statusUpdate/status/timestamp", "{event}");
    assert_eq!(shape(stamp), "dddd-dd-ddTdd:dd:dd.dddZ", "{event}");
    assert!(lies_between(stamp, before, after), "{event}");

    let answered = cancelled["result"]["status"]["timestamp"].as_str().unwrap();
    assert_eq!(shape(answered), "dddd-dd-ddTdd:dd:dd.dddZ", "{cancelled}");
    assert!(lies_between(answered, before, after), "{cancelled}");
}
