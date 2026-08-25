// ABOUTME: prescribe_workout end-to-end — MCP tool → authed provider → Intervals.icu calendar event
// ABOUTME: Intervals.icu is stubbed on loopback; asserts the event id, the audit row, and the bytes on the wire
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "provider-intervals-icu")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! End-to-end tests for the one path that writes to an athlete's training
//! calendar.
//!
//! This file is named for a push it did not perform until now: it used to be a
//! `PrescribedWorkoutRepository` round-trip over a hand-built row whose status
//! the test itself set to `queued`, which is exactly the shape a stub passes.
//! (That round-trip still runs, in `prescribed_workouts_repo_test.rs`.) The
//! tool never built a provider, so no test anywhere exercised tool → provider,
//! and `prescribe_workout` advertised a push it never made
//! ([carnet#100](https://github.com/dravr-ai/dravr-carnet/issues/100)).
//!
//! Every assertion here is one a returns-queued stub fails: a provider-issued
//! event id, an audit row carrying it, and the request bytes Intervals.icu
//! actually received.

mod common;

use std::sync::Arc;

use anyhow::Result;
use chrono::Utc;
use pierre_core::constants::oauth::INTERVALS_ICU;
use pierre_core::models::{ConnectionType, PrescribedWorkout, TenantId, UserOAuthToken};
use pierre_providers::intervals_icu_provider::default_config;
use pierre_providers::ProviderRegistry;
use pierre_tool_runtime::protocols::{UniversalRequest, UniversalResponse, UniversalToolExecutor};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use uuid::Uuid;

/// The athlete id the stubbed account is linked under — it addresses the
/// athlete-scoped URL path, so the tests can assert on it.
const ATHLETE_ID: &str = "i123456";

/// Loopback stub standing in for Intervals.icu, serving every connection with
/// the same answer and recording each full request (head + body) it received.
struct Stub {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}

impl Stub {
    /// Every request this stub has served so far, in order.
    async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }

    /// The single request the stub served. Fails loudly on zero or many, so a
    /// test can never mistake "the provider was never called" for a pass.
    async fn only_request(&self) -> String {
        let requests = self.requests().await;
        assert_eq!(
            requests.len(),
            1,
            "expected exactly one call to Intervals.icu, got {}",
            requests.len()
        );
        requests[0].clone()
    }
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

/// Read one complete HTTP request (head plus any `Content-Length` body) off
/// `socket`.
///
/// Reading only to the blank line would capture the head and drop the JSON the
/// provider posted — which is the half these tests are here to pin.
async fn read_request(socket: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut buf = [0_u8; 1024];
    let mut head_len = None;
    loop {
        let n = socket.read(&mut buf).await.expect("read request");
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        if head_len.is_none() {
            if let Some(pos) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                head_len = Some(pos + 4);
            }
        }
        if let Some(head_len) = head_len {
            let head = String::from_utf8_lossy(&raw[..head_len]).into_owned();
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if raw.len() >= head_len + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&raw).into_owned()
}

/// Stand up a loopback stub answering every request with `status_line` + `body`.
async fn stub(status_line: &'static str, body: &'static str) -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stub");
    let addr = listener.local_addr().expect("stub addr");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&requests);
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let request = read_request(&mut socket).await;
            recorded.lock().await.push(request);
            let response = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write response");
            socket.flush().await.expect("flush response");
        }
    });
    Stub {
        base_url: format!("http://{addr}"),
        requests,
        handle,
    }
}

/// An athlete with a connected provider, and the executor that serves them.
struct Fixture {
    executor: Arc<UniversalToolExecutor>,
    user_id: Uuid,
    tenant: TenantId,
}

impl Fixture {
    fn tenant_str(&self) -> String {
        self.tenant.to_string()
    }

    /// Every prescription recorded for this athlete, newest first.
    async fn prescriptions(&self) -> Vec<PrescribedWorkout> {
        self.executor
            .resources
            .repos()
            .prescribed_workouts
            .list_prescribed_workouts(self.tenant, self.user_id, 10)
            .await
            .expect("list prescriptions")
    }

    async fn prescribe(&self, params: Value) -> Result<UniversalResponse> {
        Ok(self
            .executor
            .execute_tool(UniversalRequest {
                tool_name: "prescribe_workout".to_owned(),
                parameters: params,
                user_id: self.user_id.to_string(),
                protocol: "test".to_owned(),
                tenant_id: Some(self.tenant_str()),
                progress_token: None,
                cancellation_token: None,
                progress_reporter: None,
            })
            .await?)
    }
}

/// Build an executor whose Intervals.icu provider points at `base_url`, with an
/// athlete linked to it exactly the way the link route links one: an API-key
/// token row carrying the athlete id, plus a `Manual` connection row.
///
/// `connect_intervals` off leaves the athlete on a Strava connection only —
/// enough to clear the dispatch chokepoint (which asks whether ANY data source
/// exists) so the tool's own "connect Intervals.icu" refusal is what gets
/// tested, rather than the pre-dispatch one.
async fn fixture(base_url: &str, connect_intervals: bool) -> Result<Fixture> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;

    let mut config = default_config();
    config.api_base_url = base_url.to_owned();
    let mut registry = ProviderRegistry::new();
    registry.set_default_config(INTERVALS_ICU, config);
    let mut context = (*resources).clone();
    context.fitness.provider_registry = Arc::new(registry);
    let executor = Arc::new(UniversalToolExecutor::new(Arc::new(context)));
    let resources = &executor.resources;

    let email = format!("prescribe_test_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.repos().tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user owns a tenant");

    let (provider, connection_type) = if connect_intervals {
        (INTERVALS_ICU, ConnectionType::Manual)
    } else {
        ("strava", ConnectionType::OAuth)
    };
    if connect_intervals {
        let now = Utc::now();
        resources
            .repos()
            .oauth_tokens
            .upsert_token(&UserOAuthToken {
                id: Uuid::new_v4().to_string(),
                user_id,
                tenant_id: tenant.to_string(),
                provider: INTERVALS_ICU.to_owned(),
                access_token: "test-api-key".to_owned(),
                refresh_token: None,
                token_type: "api_key".to_owned(),
                expires_at: None,
                scope: None,
                provider_user_id: Some(ATHLETE_ID.to_owned()),
                oauth_app_client_id: None,
                created_at: now,
                updated_at: now,
            })
            .await?;
    }
    resources
        .repos()
        .provider_connections
        .register_connection(user_id, tenant, provider, &connection_type, None)
        .await?;

    Ok(Fixture {
        executor: Arc::clone(&executor),
        user_id,
        tenant,
    })
}

/// The structured session from the live conversation that surfaced carnet#100 —
/// a trail session no cornerstone slug can express.
fn trail_session() -> Value {
    json!({
        "name": "Trail technique — montées 55 min",
        "sport": "run",
        "intensity_distribution": "polarized",
        "structure": [
            {
                "label": "Trail continu",
                "duration_seconds": 3300,
                "target_zone": "Z2",
                "note": "montées/descentes continues, Z2 stable",
            }
        ],
    })
}

#[tokio::test]
async fn prescribing_a_cornerstone_creates_the_calendar_event_and_records_its_id() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":987654}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    let resp = fixture
        .prescribe(json!({ "template_slug": "long_run_z2", "date": "2026-09-15" }))
        .await?;
    assert!(resp.success, "prescribe must succeed: {:?}", resp.error);
    let result = resp.result.expect("result payload");

    // The event id is the provider's, not ours — a tool that only wrote a row
    // has nothing to put here.
    assert_eq!(result["provider_event_id"].as_str(), Some("987654"));
    assert_eq!(result["status"].as_str(), Some("pushed"));
    assert_eq!(result["template_slug"].as_str(), Some("long_run_z2"));
    assert_eq!(result["scheduled_for"].as_str(), Some("2026-09-15"));
    assert_eq!(result["duration_minutes"].as_u64(), Some(90));

    let rows = fixture.prescriptions().await;
    assert_eq!(rows.len(), 1, "one prescription recorded");
    assert_eq!(rows[0].status, "pushed");
    assert_eq!(rows[0].provider_event_id.as_deref(), Some("987654"));
    assert_eq!(rows[0].provider, "intervals_icu");
    assert_eq!(rows[0].prescribed_for_date.to_string(), "2026-09-15");

    // And the athlete's actual calendar was addressed, on the date asked for.
    let request = stub.only_request().await;
    assert!(
        request.starts_with(&format!("POST /api/v1/athlete/{ATHLETE_ID}/events ")),
        "the push must POST to the athlete's events collection; got: {request}"
    );
    assert!(
        request.contains("2026-09-15T00:00:00"),
        "the event must carry the prescribed date; got: {request}"
    );
    assert!(
        request.contains("Long Run"),
        "the event must carry the workout name; got: {request}"
    );
    Ok(())
}

#[tokio::test]
async fn a_refused_push_records_a_failed_row_and_reports_the_failure() -> Result<()> {
    let stub = stub("HTTP/1.1 500 Internal Server Error", "{}").await;
    let fixture = fixture(&stub.base_url, true).await?;

    let err = fixture
        .prescribe(json!({ "template_slug": "vo2_5x3", "date": "2026-09-16" }))
        .await
        .expect_err("a refused push must not report success");
    let rendered = format!("{err:?}");
    assert!(
        rendered.contains("500") || rendered.to_lowercase().contains("intervals"),
        "the error must name the provider failure; got: {rendered}"
    );

    // The attempt is still on the record — and it is honest about the outcome.
    let rows = fixture.prescriptions().await;
    assert_eq!(rows.len(), 1, "a refused attempt is still audited");
    assert_eq!(rows[0].status, "failed");
    assert!(
        rows[0].provider_event_id.is_none(),
        "a failed push has no event id to record"
    );
    assert_eq!(rows[0].template_slug, "vo2_5x3");
    Ok(())
}

#[tokio::test]
async fn an_inline_session_is_stored_and_pushed_with_its_coaching_cue() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":424242}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    let resp = fixture
        .prescribe(json!({ "session": trail_session(), "date": "2026-09-17" }))
        .await?;
    assert!(resp.success, "session push must succeed: {:?}", resp.error);
    let result = resp.result.expect("result payload");
    assert_eq!(result["status"].as_str(), Some("pushed"));
    assert_eq!(result["provider_event_id"].as_str(), Some("424242"));
    // 3300s rounds up to 55 minutes, derived from the step — never supplied.
    assert_eq!(result["duration_minutes"].as_u64(), Some(55));

    // The session became a template this athlete can be prescribed again by slug.
    let slug = result["template_slug"]
        .as_str()
        .expect("template_slug")
        .to_owned();
    assert_eq!(slug, "trail_technique_montées_55_min");
    let stored = fixture
        .executor
        .resources
        .repos()
        .workout_templates
        .get_user_workout_template(fixture.tenant, fixture.user_id, &slug)
        .await?
        .expect("the inline session is persisted as a user template");
    assert_eq!(stored.duration_minutes, 55);
    assert_eq!(stored.structure.len(), 1);
    assert!(!stored.is_compiled_in);
    assert_eq!(stored.tenant_id, Some(fixture.tenant.as_uuid()));

    // The coach's cue reaches the athlete's calendar, not just our database.
    let request = stub.only_request().await;
    assert!(
        request.contains("montées/descentes continues"),
        "the step note must reach the calendar entry; got: {request}"
    );
    Ok(())
}

#[tokio::test]
async fn re_prescribing_the_same_session_reuses_one_template_row() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":555}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    for date in ["2026-09-18", "2026-09-25"] {
        let resp = fixture
            .prescribe(json!({ "session": trail_session(), "date": date }))
            .await?;
        assert!(resp.success, "push must succeed: {:?}", resp.error);
    }

    // Two calendar entries (one per date), one library row — the slug is unique
    // per athlete, so a second mint would have violated that index outright.
    assert_eq!(stub.requests().await.len(), 2, "one push per prescription");
    assert_eq!(fixture.prescriptions().await.len(), 2);
    let templates = fixture
        .executor
        .resources
        .repos()
        .workout_templates
        .list_user_workout_templates(fixture.tenant, fixture.user_id)
        .await?;
    assert_eq!(
        templates.len(),
        1,
        "the same named session must converge on one stored template"
    );
    Ok(())
}

#[tokio::test]
async fn a_slug_that_names_nothing_is_rejected_before_any_push() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":1}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    let err = fixture
        .prescribe(json!({ "template_slug": "not_a_real_template", "date": "2026-09-19" }))
        .await
        .expect_err("unknown slug must be rejected");
    assert!(
        format!("{err:?}").contains("not_a_real_template"),
        "the rejection must name the slug: {err:?}"
    );
    assert!(
        stub.requests().await.is_empty(),
        "an unresolvable slug must never reach the provider"
    );
    assert!(fixture.prescriptions().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn the_workout_must_be_named_exactly_once() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":1}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    let neither = fixture
        .prescribe(json!({ "date": "2026-09-20" }))
        .await
        .expect_err("a prescription with no workout must be rejected");
    assert!(
        format!("{neither:?}").contains("template_slug"),
        "the rejection must say what to send: {neither:?}"
    );

    let both = fixture
        .prescribe(json!({
            "template_slug": "long_run_z2",
            "session": trail_session(),
            "date": "2026-09-20",
        }))
        .await
        .expect_err("an ambiguous prescription must be rejected");
    assert!(format!("{both:?}").contains("not both"), "got: {both:?}");

    assert!(stub.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_session_with_no_steps_is_rejected() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":1}"#).await;
    let fixture = fixture(&stub.base_url, true).await?;

    let err = fixture
        .prescribe(json!({
            "session": {
                "name": "Empty",
                "sport": "run",
                "intensity_distribution": "polarized",
                "structure": [],
            },
            "date": "2026-09-21",
        }))
        .await
        .expect_err("a session with no steps has no duration to prescribe");
    assert!(
        format!("{err:?}").contains("at least one step"),
        "got: {err:?}"
    );
    assert!(stub.requests().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn an_athlete_without_intervals_icu_is_told_to_connect_it() -> Result<()> {
    let stub = stub("HTTP/1.1 200 OK", r#"{"id":1}"#).await;
    // Strava-connected only: past the dispatch chokepoint, short of a calendar.
    let fixture = fixture(&stub.base_url, false).await?;

    let resp = fixture
        .prescribe(json!({ "template_slug": "long_run_z2", "date": "2026-09-22" }))
        .await;
    let rendered = match resp {
        Ok(response) => {
            assert!(
                !response.success,
                "a calendar-less athlete must not succeed"
            );
            format!("{:?}", response.error)
        }
        Err(err) => format!("{err:?}"),
    };
    assert!(
        rendered.to_lowercase().contains("intervals"),
        "the refusal must name the account to connect; got: {rendered}"
    );
    assert!(
        stub.requests().await.is_empty(),
        "nothing may be pushed without a linked account"
    );
    assert!(fixture.prescriptions().await.is_empty());
    Ok(())
}
