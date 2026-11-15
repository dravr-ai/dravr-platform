// ABOUTME: carnet#1508 — a sign-in a provider's robot check parked is taken over by the athlete through a relay
// ABOUTME: Pins the takeover link, the token's reach, the relay both ways, the concluded sign-in, and the cancel on close

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! ADR-034: a login that names its window's attempt asks sciotte for a
//! takeover. One a captcha parks answers `takeover_required` with a link to
//! `/sign-in-check` carrying a takeover token in its fragment; the page opens
//! `/ws/providers/sciotte/takeover?token=…`, and the platform relays it to
//! sciotte's own takeover socket. These tests run the platform's real routes
//! against a loopback stand-in of the scraper service (a test double, per the
//! repo's mock rule), on a thread of its own so it outlives each test's
//! runtime.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::garmin_sign_in_test::server_with_garmin_oauth;
use crate::helpers;

use std::net::SocketAddr;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use axum::extract::ws::{Message as AxumMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use common::{create_test_user_with_email, generate_test_token};
use futures_util::{SinkExt, StreamExt};
use helpers::axum_test::AxumTestRequest;
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::constants::oauth_providers::SCIOTTE_GARMIN;
use pierre_core::models::TenantId;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_takeover_link_token, MintTakeoverTokenArgs, TakeoverLink,
    CONNECT_PROVIDER,
};
use pierre_routes_auth::AuthRoutes;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Builder;
use tokio::time::{sleep, timeout};
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

/// Logins whose email starts with this are parked at a robot check when the
/// request asks for a takeover.
const CHALLENGED: &str = "challenged";

/// Logins whose email starts with this park a flow the service has let go
/// of by the time its takeover opens.
const GONE: &str = "challenged-gone";

/// Logins whose email starts with this park a flow whose takeover socket
/// the service closes, with no answer, on the viewer's first input.
const DROPPED: &str = "challenged-drop";

/// The flow the stand-in parks for a takeover, per login email.
fn flow_of(email: &str) -> String {
    let id = &hex::encode(Sha256::digest(email.as_bytes()))[..16];
    if email.starts_with(GONE) {
        format!("flow-gone-{id}")
    } else if email.starts_with(DROPPED) {
        format!("flow-drop-{id}")
    } else {
        format!("flow-{id}")
    }
}

/// The longest any one wait here may take.
const WAIT: Duration = Duration::from_secs(10);

type Viewer = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ============================================================================
// The scraper service stand-in
// ============================================================================

/// What the stand-in received.
#[derive(Default)]
struct Seen {
    /// Every login: its email and whether it asked for a takeover.
    logins: Mutex<Vec<(String, bool)>>,
    /// Every takeover input message, per flow, as received.
    inputs: Mutex<Vec<(String, Value)>>,
    /// Every cancel key a cancel named.
    cancels: Mutex<Vec<String>>,
}

static SCRAPER: OnceLock<(Arc<Seen>, String)> = OnceLock::new();

fn scraper() -> &'static (Arc<Seen>, String) {
    SCRAPER.get_or_init(|| {
        let seen = Arc::new(Seen::default());
        let (sender, receiver) = mpsc::channel();
        let state = Arc::clone(&seen);
        thread::spawn(move || {
            let runtime = Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                sender
                    .send(format!("http://{}", listener.local_addr().unwrap()))
                    .unwrap();
                axum::serve(listener, stand_in(state)).await.unwrap();
            });
        });
        (seen, receiver.recv().unwrap())
    })
}

fn stand_in(seen: Arc<Seen>) -> Router {
    Router::new()
        .route("/auth/login-with-credentials", post(login))
        .route("/auth/takeover/{flow_id}", get(takeover))
        .route("/auth/sessions/{id}/export", get(export))
        .route("/auth/cancel-login", post(cancel))
        .with_state(seen)
}

async fn login(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Response {
    let email = body["email"].as_str().unwrap_or_default().to_owned();
    let takeover = body["takeover"].as_bool().unwrap_or(false);
    seen.logins.lock().unwrap().push((email.clone(), takeover));
    if email.starts_with(CHALLENGED) && takeover {
        return Json(json!({
            "status": "takeover_required",
            "flow_id": flow_of(&email),
            "provider": "garmin"
        }))
        .into_response();
    }
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "login_challenge", "message": "Garmin put a challenge in front of the login" })),
    )
        .into_response()
}

/// The takeover socket: a status, one frame, then each input recorded; a
/// pointer `up` signs the athlete in. A flow whose name says `gone` is
/// refused before the upgrade, as sciotte refuses one it does not hold; one
/// whose name says `drop` closes with no answer on its first input.
async fn takeover(
    State(seen): State<Arc<Seen>>,
    Path(flow_id): Path<String>,
    upgrade: WebSocketUpgrade,
) -> Response {
    if flow_id.contains("gone") {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no_pending_login" })),
        )
            .into_response();
    }
    upgrade.on_upgrade(move |socket| drive(seen, flow_id, socket))
}

async fn drive(seen: Arc<Seen>, flow_id: String, mut socket: WebSocket) {
    let status = json!({"type": "status", "state": "ready", "width": 400, "height": 700});
    let _sent = socket
        .send(AxumMessage::Text(status.to_string().into()))
        .await;
    let _sent = socket
        .send(AxumMessage::Binary(vec![0xFF, 0xD8, 0xFF, 0xD9].into()))
        .await;
    while let Some(Ok(AxumMessage::Text(text))) = socket.recv().await {
        let input: Value = serde_json::from_str(&text).unwrap();
        seen.inputs
            .lock()
            .unwrap()
            .push((flow_id.clone(), input.clone()));
        if flow_id.contains("drop") {
            let _closed = socket.send(AxumMessage::Close(None)).await;
            return;
        }
        if input["type"] == "pointer" && input["action"] == "up" {
            let outcome = json!({
                "type": "outcome",
                "status_code": 200,
                "body": {
                    "status": "authenticated",
                    "session_id": format!("sess-{flow_id}"),
                    "cookie_count": 1,
                    "provider": "garmin"
                }
            });
            let _sent = socket
                .send(AxumMessage::Text(outcome.to_string().into()))
                .await;
            let _closed = socket.send(AxumMessage::Close(None)).await;
            return;
        }
    }
}

async fn export(Path(id): Path<String>) -> Json<Value> {
    Json(json!({
        "provider": "garmin",
        "session": {
            "session_id": id,
            "cookies": [{
                "name": "SESSIONID", "value": "takeover-cookie", "domain": ".garmin.com",
                "path": "/", "http_only": true, "secure": true
            }],
            "created_at": "2026-11-12T12:00:00Z",
            "expires_at": null
        }
    }))
}

async fn cancel(State(seen): State<Arc<Seen>>, Json(body): Json<Value>) -> Json<Value> {
    seen.cancels
        .lock()
        .unwrap()
        .push(body["cancel_key"].as_str().unwrap_or_default().to_owned());
    Json(json!({ "cancelled_running": true, "released_parked": false }))
}

// ============================================================================
// The platform under test
// ============================================================================

/// One athlete on a platform served over loopback, so its socket is real.
struct Athlete {
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    bearer: String,
    platform: SocketAddr,
}

impl Athlete {
    async fn new() -> Self {
        let resources = common::create_test_server_resources_with_config(
            common::test_server_config_with_sciotte_usr_pwd_garmin(&scraper().1),
        )
        .await
        .unwrap();
        Self::on(resources).await
    }

    /// An athlete of a platform whose Garmin OAuth is served too, so a
    /// sign-in goes on to the grant it owes (carnet#1494).
    async fn with_garmin_oauth() -> Self {
        Self::on(server_with_garmin_oauth(&scraper().1).await).await
    }

    async fn on(resources: Arc<ServerContext>) -> Self {
        let (user_id, user) = create_test_user_with_email(
            &resources.agent.database,
            &format!("athlete-{}@example.com", Uuid::new_v4()),
        )
        .await
        .unwrap();
        if let Some(version) = provider_terms_version(SCIOTTE_GARMIN) {
            resources
                .common
                .repos
                .users
                .record_provider_terms(user_id, SCIOTTE_GARMIN, version)
                .await
                .unwrap();
        }
        let tenant_id = resources
            .common
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .unwrap()
            .first()
            .expect("user has a tenant")
            .id;
        let bearer = format!("Bearer {}", generate_test_token(&resources, &user).await);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let platform = listener.local_addr().unwrap();
        let app = AuthRoutes::routes(resources.auth_routes_context());
        tokio::spawn(async move { axum::serve(listener, app).await });
        Self {
            resources,
            user_id,
            tenant_id,
            bearer,
            platform,
        }
    }

    async fn login(&self, email: &str, attempt_id: Option<&str>) -> (u16, Value) {
        let mut body = json!({
            "email": email,
            "password": "not-a-real-password",
            "target": "garmin",
            "tos_consent": true,
        });
        if let Some(attempt_id) = attempt_id {
            body["attempt_id"] = json!(attempt_id);
        }
        let resp = AxumTestRequest::post("/api/providers/sciotte/login")
            .header("authorization", &self.bearer)
            .json(&body)
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        let status = resp.status();
        let text = resp.text();
        (
            status,
            serde_json::from_str(&text).unwrap_or_else(|_| panic!("{text}")),
        )
    }

    async fn cancel(&self, attempt_id: &str) {
        let resp = AxumTestRequest::post("/api/providers/sciotte/cancel")
            .header("authorization", &self.bearer)
            .json(&json!({ "attempt_id": attempt_id }))
            .send(AuthRoutes::routes(self.resources.auth_routes_context()))
            .await;
        assert_eq!(resp.status(), 200, "{}", resp.text());
    }

    /// A takeover login; its takeover token, read off the link's fragment.
    async fn park(&self, email: &str, attempt_id: &str) -> String {
        let (status, body) = self.login(email, Some(attempt_id)).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["status"], "takeover_required", "{body}");
        assert!(
            body.get("flow_id").is_none(),
            "the flow id never reaches the client: {body}"
        );
        let url = body["takeover_url"].as_str().unwrap();
        let (page, token) = url
            .split_once("#token=")
            .expect("the token rides in the fragment");
        assert!(page.ends_with("/sign-in-check"), "{url}");
        urlencoding::decode(token).unwrap().into_owned()
    }

    async fn open(&self, token: &str) -> Result<Viewer, WsError> {
        connect_async(format!(
            "ws://{}/ws/providers/sciotte/takeover?token={}",
            self.platform,
            urlencoding::encode(token)
        ))
        .await
        .map(|(socket, _)| socket)
    }

    async fn holds_garmin_session(&self) -> bool {
        self.resources
            .common
            .repos
            .oauth_tokens
            .get_token(self.user_id, self.tenant_id, SCIOTTE_GARMIN)
            .await
            .unwrap()
            .is_some()
    }

    fn cancel_key(&self, attempt_id: &str) -> String {
        hex::encode(Sha256::digest(format!(
            "{}:{}:{attempt_id}",
            self.tenant_id.as_uuid(),
            self.user_id
        )))
    }
}

fn email(prefix: &str) -> String {
    format!("{prefix}-{}@example.com", Uuid::new_v4())
}

fn attempt() -> String {
    Uuid::new_v4().to_string()
}

/// Read until a text event of `kind`, counting the frames before it.
async fn read_event(viewer: &mut Viewer, kind: &str) -> (Value, usize) {
    let mut frames = 0;
    let read = async {
        while let Some(message) = viewer.next().await {
            match message.unwrap() {
                Message::Binary(frame) => {
                    assert_eq!(
                        &frame[..2],
                        &[0xFF, 0xD8],
                        "a relayed frame is the JPEG the service sent"
                    );
                    frames += 1;
                }
                Message::Text(text) => {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["type"] == kind {
                        return event;
                    }
                }
                Message::Close(_) => panic!("closed before a {kind} event"),
                _ => {}
            }
        }
        panic!("ended before a {kind} event");
    };
    let event = timeout(WAIT, read).await.expect("the event arrives");
    (event, frames)
}

async fn send(viewer: &mut Viewer, input: Value) {
    viewer.send(Message::text(input.to_string())).await.unwrap();
}

fn inputs_of(flow_id: &str) -> Vec<Value> {
    scraper()
        .0
        .inputs
        .lock()
        .unwrap()
        .iter()
        .filter(|(flow, _)| flow == flow_id)
        .map(|(_, input)| input.clone())
        .collect()
}

async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        sleep(Duration::from_millis(50)).await;
    }
    panic!("{what}");
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn a_window_with_an_attempt_asks_for_a_takeover_and_one_without_does_not() {
    let athlete = Athlete::new().await;
    let with = email(CHALLENGED);
    athlete.park(&with, &attempt()).await;

    let without = email(CHALLENGED);
    let (status, body) = athlete.login(&without, None).await;
    assert_ne!(
        status, 200,
        "a login that cannot cancel keeps the challenge answer: {body}"
    );

    let logins = scraper().0.logins.lock().unwrap().clone();
    assert!(
        logins.contains(&(with, true)),
        "the login naming an attempt asked for a takeover"
    );
    assert!(
        logins.contains(&(without, false)),
        "the login naming no attempt asked for no takeover"
    );
}

#[tokio::test]
async fn the_athlete_signs_in_through_the_relay_and_the_session_is_stored() {
    let athlete = Athlete::new().await;
    let login = email(CHALLENGED);
    let token = athlete.park(&login, &attempt()).await;
    let mut viewer = athlete.open(&token).await.expect("the relay upgrades");

    let (status, _) = read_event(&mut viewer, "status").await;
    assert_eq!(
        status["width"], 400,
        "the service's status reaches the viewer: {status}"
    );
    // Not a TakeoverInput: dropped by the relay, never sent on.
    send(&mut viewer, json!({"type": "script", "code": "alert(1)"})).await;
    send(&mut viewer, json!({"type": "text", "text": "aa"})).await;
    send(
        &mut viewer,
        json!({"type": "pointer", "action": "down", "x": 30.0, "y": 310.0}),
    )
    .await;
    send(
        &mut viewer,
        json!({"type": "pointer", "action": "up", "x": 30.0, "y": 310.0}),
    )
    .await;

    let (outcome, frames) = read_event(&mut viewer, "outcome").await;
    assert!(frames >= 1, "the frame was relayed");
    assert_eq!(outcome["status_code"], 200, "{outcome}");
    assert_eq!(outcome["body"]["status"], "connected", "{outcome}");
    assert!(
        outcome["body"].get("session_id").is_none(),
        "the service's session id never reaches the viewer: {outcome}"
    );
    assert!(
        athlete.holds_garmin_session().await,
        "the session is stored"
    );

    let inputs = inputs_of(&flow_of(&login));
    assert_eq!(
        inputs.len(),
        3,
        "only the takeover inputs were relayed: {inputs:?}"
    );
    assert_eq!(
        inputs[0],
        json!({"type": "text", "text": "aa"}),
        "the typed text reaches the service exact"
    );
}

/// Sign the athlete in through an open takeover: the stand-in's pointer `up`
/// authenticates. The step answer the viewer receives.
async fn sign_in_through(viewer: &mut Viewer) -> Value {
    read_event(viewer, "status").await;
    for action in ["down", "up"] {
        send(
            viewer,
            json!({"type": "pointer", "action": action, "x": 30.0, "y": 310.0}),
        )
        .await;
    }
    let (outcome, _) = read_event(viewer, "outcome").await;
    assert_eq!(outcome["status_code"], 200, "{outcome}");
    assert_eq!(outcome["body"]["status"], "connected", "{outcome}");
    outcome["body"].clone()
}

/// A Garmin sign-in a takeover concluded goes on to the grant it owes, as
/// the same sign-in over HTTP does (carnet#1494): the app starts it from the
/// provider it names.
#[tokio::test]
async fn a_takeover_sign_in_goes_on_to_the_garmin_grant_it_owes() {
    let athlete = Athlete::with_garmin_oauth().await;
    let token = athlete.park(&email(CHALLENGED), &attempt()).await;
    let mut viewer = athlete.open(&token).await.expect("the relay upgrades");

    let answer = sign_in_through(&mut viewer).await;
    assert_eq!(
        answer["oauth_follow_on"]["provider"], GARMIN,
        "bug: a takeover sign-in does not go on to its grant: {answer}"
    );
    assert!(
        answer["oauth_follow_on"].get("authorization_url").is_none(),
        "the app starts the authorization its own way: {answer}"
    );
}

/// The hosted connect picker starts a grant with its own token, but a
/// takeover concludes on the takeover page, where that page is gone: the
/// answer carries Garmin's authorization URL itself.
#[tokio::test]
async fn a_hosted_takeover_sign_in_carries_its_grants_authorization_url() {
    let athlete = Athlete::with_garmin_oauth().await;
    let attempt_id = attempt();
    athlete.park(&email(CHALLENGED), &attempt_id).await;
    // The takeover token the picker's sign-in would have been handed.
    let token = mint_takeover_link_token(
        &MintTakeoverTokenArgs {
            user_id: athlete.user_id,
            tenant_id: athlete.tenant_id.as_uuid(),
            provider: SCIOTTE_GARMIN,
            attempt_id: &attempt_id,
            link: Some(TakeoverLink {
                channel: "telegram".to_owned(),
                channel_thread: None,
                target: CONNECT_PROVIDER.to_owned(),
            }),
        },
        &athlete.resources.auth_routes_context().admin_jwt_secret,
    )
    .unwrap();
    let mut viewer = athlete.open(&token).await.expect("the relay upgrades");

    let answer = sign_in_through(&mut viewer).await;
    assert_eq!(answer["oauth_follow_on"]["provider"], GARMIN, "{answer}");
    let url = answer["oauth_follow_on"]["authorization_url"]
        .as_str()
        .unwrap_or_else(|| panic!("bug: the takeover page has no way on to the grant: {answer}"));
    assert!(
        url.contains("client_id="),
        "bug: the follow-on is not Garmin's authorization: {url}"
    );
}

#[tokio::test]
async fn the_viewer_closing_cancels_the_attempt() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let token = athlete.park(&email(CHALLENGED), &attempt_id).await;
    let mut viewer = athlete.open(&token).await.unwrap();
    read_event(&mut viewer, "status").await;
    viewer.close(None).await.unwrap();

    let key = athlete.cancel_key(&attempt_id);
    eventually("the relay cancelled the attempt on the service", || {
        scraper().0.cancels.lock().unwrap().contains(&key)
    })
    .await;
    assert!(
        !athlete.holds_garmin_session().await,
        "a closed takeover links nothing"
    );
}

#[tokio::test]
async fn a_cancelled_attempts_sign_in_is_refused_even_through_the_relay() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let token = athlete.park(&email(CHALLENGED), &attempt_id).await;
    let mut viewer = athlete.open(&token).await.unwrap();
    read_event(&mut viewer, "status").await;

    // The window's cancel lands on an instance the service never told.
    athlete.cancel(&attempt_id).await;
    send(
        &mut viewer,
        json!({"type": "pointer", "action": "up", "x": 1.0, "y": 1.0}),
    )
    .await;

    let (outcome, _) = read_event(&mut viewer, "outcome").await;
    assert_eq!(outcome["status_code"], 409, "{outcome}");
    assert!(
        !athlete.holds_garmin_session().await,
        "a cancelled attempt links nothing"
    );
}

#[tokio::test]
async fn a_token_that_is_not_a_takeover_token_opens_nothing() {
    let athlete = Athlete::new().await;
    athlete.park(&email(CHALLENGED), &attempt()).await;
    let connect = mint_connect_link_token(
        athlete.user_id,
        athlete.tenant_id.as_uuid(),
        "slack",
        None,
        &athlete.resources.auth_routes_context().admin_jwt_secret,
    )
    .unwrap();
    for token in [connect.as_str(), "not-a-token"] {
        match athlete.open(token).await {
            Err(WsError::Http(response)) => assert_eq!(
                response.status(),
                401,
                "a non-takeover token is refused as unauthenticated"
            ),
            other => panic!(
                "a non-takeover token is refused 401, got {:?}",
                other.map(|_| ())
            ),
        }
    }
}

#[tokio::test]
async fn a_token_for_someone_without_a_parked_sign_in_reaches_no_flow() {
    let athlete = Athlete::new().await;
    athlete.park(&email(CHALLENGED), &attempt()).await;
    // A takeover token for another athlete, who parked nothing.
    let stranger = Uuid::new_v4();
    let token = mint_takeover_link_token(
        &MintTakeoverTokenArgs {
            user_id: stranger,
            tenant_id: athlete.tenant_id.as_uuid(),
            provider: SCIOTTE_GARMIN,
            attempt_id: &attempt(),
            link: None,
        },
        &athlete.resources.auth_routes_context().admin_jwt_secret,
    )
    .unwrap();
    match athlete.open(&token).await {
        Err(WsError::Http(response)) => assert_eq!(
            response.status(),
            400,
            "a token for someone with no parked sign-in reaches no flow"
        ),
        other => panic!("no parked flow is a 400, got {:?}", other.map(|_| ())),
    }
}

#[tokio::test]
async fn a_flow_the_service_let_go_ends_the_check_before_any_socket() {
    let athlete = Athlete::new().await;
    let token = athlete.park(&email(GONE), &attempt()).await;
    match athlete.open(&token).await {
        Err(WsError::Http(response)) => assert_eq!(
            response.status(),
            400,
            "a flow the service let go ends the check"
        ),
        other => panic!(
            "a flow the service let go is a 400, got {:?}",
            other.map(|_| ())
        ),
    }
    // A second try is refused the same way.
    match athlete.open(&token).await {
        Err(WsError::Http(response)) => assert_eq!(
            response.status(),
            400,
            "the flow the service let go is forgotten"
        ),
        other => panic!(
            "a second try at a forgotten flow is a 400, got {:?}",
            other.map(|_| ())
        ),
    }
}

#[tokio::test]
async fn the_service_ending_with_no_answer_cancels_the_attempt_and_says_it_ended() {
    let athlete = Athlete::new().await;
    let attempt_id = attempt();
    let token = athlete.park(&email(DROPPED), &attempt_id).await;
    let mut viewer = athlete.open(&token).await.expect("the relay upgrades");
    read_event(&mut viewer, "status").await;
    send(
        &mut viewer,
        json!({"type": "pointer", "action": "down", "x": 1.0, "y": 1.0}),
    )
    .await;

    let (outcome, _) = read_event(&mut viewer, "outcome").await;
    assert_eq!(
        outcome["status_code"], 400,
        "an ended takeover is a refusal: {outcome}"
    );
    assert_eq!(
        outcome["body"]["details"]["reason"], "takeover_ended",
        "the viewer is told the takeover ended: {outcome}"
    );
    let key = athlete.cancel_key(&attempt_id);
    eventually("the relay cancelled the attempt on the service", || {
        scraper().0.cancels.lock().unwrap().contains(&key)
    })
    .await;
    assert!(
        !athlete.holds_garmin_session().await,
        "a takeover the service ended links nothing"
    );
}
