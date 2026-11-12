// ABOUTME: The login takeover relay: the athlete's viewer on one side, sciotte's takeover socket on the other
// ABOUTME: Verifies the takeover token, binds the caller's own parked flow, and concludes the sign-in as an HTTP step does
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `GET /ws/providers/sciotte/takeover?token=…` (ADR-034).
//!
//! A sign-in the provider's robot check parked answers `takeover_required`
//! with a link whose fragment carries a takeover token. The page it opens
//! connects here with that token. This relay:
//!
//! - verifies the token ([`verify_takeover_link_token`]), and binds the
//!   caller to their own parked flow through the remembered flow
//!   (`require_remote_flow`): the token never names a flow id, so a caller
//!   can only ever reach the browser parked for them;
//! - opens sciotte's takeover socket under the service's identity token, and
//!   only then upgrades, so a flow that is gone answers as an HTTP refusal;
//! - relays frames out, and the viewer's input in, read as sciotte's own
//!   [`TakeoverInput`] (no second schema), at most [`MAX_INPUT_BYTES`] each and
//!   [`MAX_INPUTS_PER_SECOND`] a second;
//! - intercepts the step's outcome: the sign-in concludes as an HTTP step
//!   does (`remote_login_to_response`: the session exported and stored, the
//!   channel post-back emitted), a cancelled attempt's result is refused, and
//!   the viewer receives what `POST /api/providers/sciotte/login` would have
//!   answered — never the service's session id;
//! - cancels the attempt when either side ends without an outcome: closing
//!   the viewer ends the sign-in, and there is no reconnect.

use std::time::{Duration, Instant};

use axum::body::to_bytes;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dravr_sciotte::client::{ClientError, SciotteClient};
use dravr_sciotte::wire::{LoginOutcome, TakeoverEvent, TakeoverInput};
use dravr_tronc::service_client::{ServiceWebSocket, WebSocketMessage};
use futures::{SinkExt, StreamExt};
use pierre_core::errors::AppError;
use pierre_middleware::provider_link_token::{
    mint_takeover_link_token, verify_takeover_link_token, MintTakeoverTokenArgs, TakeoverLink,
    TakeoverTokenClaims,
};
use pierre_providers::sciotte_error::to_app_error;
use serde::Deserialize;
use serde_json::Value;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::sciotte::{
    forget_remote_flow, remember_remote_flow, remote_login_to_response, require_remote_flow,
    LoginStep,
};
use crate::sciotte_caller::LinkContext;
use crate::sciotte_cancel::{
    cancel_attempt, refuse_cancelled_attempt, refuse_cancelled_outcome, StepAttempt,
};
use crate::AuthRoutesContext;

/// The largest message a viewer may send, in bytes.
const MAX_INPUT_BYTES: usize = 4 * 1024;

/// The most input messages a viewer may send in one second; the rest of that
/// second's are dropped.
const MAX_INPUTS_PER_SECOND: u32 = 60;

/// The largest answer body read back to send on the socket.
const MAX_ANSWER_BYTES: usize = 64 * 1024;

/// `details.reason` of the answer a viewer gets when the takeover ended with
/// no outcome from the service.
const TAKEOVER_ENDED_REASON: &str = "takeover_ended";

/// Query of the takeover socket.
#[derive(Deserialize)]
pub(crate) struct TakeoverQuery {
    /// The takeover token the page read from its link's fragment.
    token: String,
}

/// Who a takeover is for, read from its verified token.
struct Caller {
    user_id: Uuid,
    tenant_id: Uuid,
    attempt: StepAttempt,
    link: Option<LinkContext>,
}

impl Caller {
    fn of(claims: TakeoverTokenClaims) -> Result<Self, AppError> {
        let user_id = Uuid::parse_str(&claims.sub)
            .map_err(|_| AppError::auth_invalid("Takeover token has an invalid user"))?;
        let tenant_id = Uuid::parse_str(&claims.tid)
            .map_err(|_| AppError::auth_invalid("Takeover token has an invalid tenant"))?;
        let attempt = StepAttempt::of_step(tenant_id, user_id, Some(&claims.att))?
            .ok_or_else(|| AppError::auth_invalid("Takeover token names no attempt"))?;
        let link = claims.link.map(|link| LinkContext {
            channel: link.channel,
            channel_thread: link.channel_thread,
            target: link.target,
            on_takeover_page: true,
        });
        Ok(Self {
            user_id,
            tenant_id,
            attempt,
            link,
        })
    }
}

/// `GET /ws/providers/sciotte/takeover?token=…`: the athlete's view of their
/// sign-in parked at a provider's robot check.
///
/// # Errors
///
/// `401` for a token that is not a valid takeover token; `400` when the
/// caller has no parked sign-in (it ended, or lapsed); `409` for an attempt
/// already cancelled; the service's refusal when the flow it held is gone.
pub(crate) async fn handle_sciotte_takeover_ws(
    State(resources): State<AuthRoutesContext>,
    Query(query): Query<TakeoverQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let claims = verify_takeover_link_token(&query.token, &resources.admin_jwt_secret)?;
    let token_provider = claims.tgt.clone();
    let caller = Caller::of(claims)?;
    refuse_cancelled_attempt(&resources.cache, Some(&caller.attempt)).await?;
    let (flow_id, provider) =
        require_remote_flow(&resources.cache, caller.tenant_id, caller.user_id).await?;
    if provider != token_provider {
        return Err(AppError::invalid_input(
            "This sign-in check is for another provider's sign-in",
        ));
    }
    let remote = resources
        .provider_registry
        .require_sciotte_client()?
        .clone();
    let service = match remote.takeover(&flow_id).await {
        Ok(service) => service,
        // The service answered for the flow: it is gone, cancelled, or
        // already another socket's. The remembered flow is left as it is: a
        // second page on the same link must not unbind the first one's
        // sign-in, and an entry for a flow that is gone ages out with it.
        Err(refused @ ClientError::Unexpected { .. }) => {
            info!(user_id = %caller.user_id, error = %refused, "Sciotte refused the takeover socket");
            return Err(AppError::invalid_input(
                "This sign-in check has ended. Try connecting again.",
            ));
        }
        // No answer at all is the service's failure, not the athlete's.
        Err(e) => return Err(to_app_error(e)),
    };
    info!(user_id = %caller.user_id, provider = %provider, "Sciotte takeover opened");
    Ok(upgrade
        .max_message_size(MAX_INPUT_BYTES)
        .on_upgrade(move |viewer| async move {
            let relay = Relay {
                resources,
                remote,
                caller,
                flow_id,
                provider,
            };
            Box::pin(relay.run(viewer, service)).await;
        }))
}

/// Answer a login the provider's robot check parked for the athlete's
/// takeover (ADR-034): remember the flow as every parked step does, and hand
/// the client the takeover page's link, which carries a takeover token in its
/// fragment. The flow id never reaches the client.
pub(crate) async fn takeover_required(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    step: LoginStep<'_>,
    flow_id: String,
    link_context: Option<&LinkContext>,
) -> Result<Response, AppError> {
    // Only a login that named an attempt asked for one.
    let LoginStep::Login {
        provider,
        attempt: Some(attempt),
    } = step
    else {
        forget_remote_flow(&resources.cache, tenant_id, user_id).await;
        return Err(AppError::internal(
            "sciotte parked a takeover the platform did not ask for",
        ));
    };
    remember_remote_flow(&resources.cache, tenant_id, user_id, flow_id, provider).await;
    let token = mint_takeover_link_token(
        &MintTakeoverTokenArgs {
            user_id,
            tenant_id,
            provider,
            attempt_id: attempt.attempt_id(),
            link: link_context.map(|link| TakeoverLink {
                channel: link.channel.clone(),
                channel_thread: link.channel_thread.clone(),
                target: link.target.clone(),
            }),
        },
        &resources.admin_jwt_secret,
    )?;
    info!(
        user_id = %user_id,
        provider = %provider,
        "Sciotte login parked at a provider challenge; handing the athlete the takeover"
    );
    Ok(Json(serde_json::json!({
        "status": "takeover_required",
        "provider": provider,
        "takeover_url": takeover_page_url(resources, &token),
    }))
    .into_response())
}

/// The web app's page a takeover opens, carrying `token` in its fragment so
/// no server log ever records it.
fn takeover_page_url(resources: &AuthRoutesContext, token: &str) -> String {
    let base = resources
        .config
        .frontend_url
        .as_deref()
        .unwrap_or(&resources.config.base_url);
    format!(
        "{base}{TAKEOVER_PAGE}#{TAKEOVER_TOKEN_KEY}={}",
        urlencoding::encode(token)
    )
}

/// The web app's page a takeover link opens.
const TAKEOVER_PAGE: &str = "/sign-in-check";

/// The fragment key the takeover link carries its token under.
const TAKEOVER_TOKEN_KEY: &str = "token";

/// One takeover's two sockets and what concluding it needs.
struct Relay {
    resources: AuthRoutesContext,
    remote: SciotteClient,
    caller: Caller,
    flow_id: String,
    provider: String,
}

/// How the relay's loop ended.
enum Ended {
    /// The service sent the step's answer.
    Outcome { status_code: u16, body: Value },
    /// The viewer closed, or its socket broke.
    ViewerLeft,
    /// The service's socket ended with no answer.
    ServiceGone,
}

/// Counts a viewer's input messages, one second at a time.
struct InputBudget {
    window: Instant,
    used: u32,
}

impl InputBudget {
    fn new() -> Self {
        Self {
            window: Instant::now(),
            used: 0,
        }
    }

    /// Whether one more message fits this second.
    fn take(&mut self) -> bool {
        if self.window.elapsed() >= Duration::from_secs(1) {
            self.window = Instant::now();
            self.used = 0;
        }
        self.used += 1;
        self.used <= MAX_INPUTS_PER_SECOND
    }
}

impl Relay {
    async fn run(self, mut viewer: WebSocket, mut service: ServiceWebSocket) {
        let ended = relay_until_end(&mut viewer, &mut service).await;
        if let Err(e) = service.close(None).await {
            debug!(error = %e, "Sciotte takeover socket was already closed");
        }
        self.finish(ended, &mut viewer).await;
        if let Err(e) = viewer.send(Message::Close(None)).await {
            debug!(error = %e, "Takeover viewer was already gone");
        }
    }

    /// Answer the viewer for how the relay ended, cancelling the sign-in when
    /// it ended with no outcome.
    async fn finish(&self, ended: Ended, viewer: &mut WebSocket) {
        match ended {
            Ended::Outcome { status_code, body } => {
                let (status_code, body) = self.conclude(status_code, body).await;
                send_answer(viewer, status_code, body).await;
            }
            Ended::ServiceGone => {
                self.cancel().await;
                let (status_code, body) = takeover_ended().await;
                send_answer(viewer, status_code, body).await;
            }
            Ended::ViewerLeft => self.cancel().await,
        }
    }

    /// The service's step answer, concluded as an HTTP step concludes it:
    /// what the viewer receives in its place.
    async fn conclude(&self, status_code: u16, body: Value) -> (u16, Value) {
        let caller = &self.caller;
        let cache = &self.resources.cache;
        if let Err(refused) = refuse_cancelled_outcome(
            cache,
            &self.remote,
            Some(&caller.attempt),
            Some(&self.flow_id),
        )
        .await
        {
            return answer_of(refused.into_response()).await;
        }
        let Ok(outcome) = serde_json::from_value::<LoginOutcome>(body) else {
            // A refusal: the flow is gone on the service's side.
            info!(
                user_id = %caller.user_id,
                status_code,
                "Sciotte ended the takeover with a refusal"
            );
            forget_remote_flow(cache, caller.tenant_id, caller.user_id).await;
            return takeover_ended().await;
        };
        let answered = remote_login_to_response(
            outcome,
            &self.remote,
            &self.resources,
            caller.user_id,
            caller.tenant_id,
            LoginStep::Continuation {
                provider: &self.provider,
                flow_id: &self.flow_id,
            },
            caller.link.as_ref(),
        )
        .await;
        match answered {
            Ok(response) => answer_of(response).await,
            Err(refused) => answer_of(refused.into_response()).await,
        }
    }

    /// End the attempt the takeover was for: its browser and its slot free.
    async fn cancel(&self) {
        let caller = &self.caller;
        info!(user_id = %caller.user_id, "Sciotte takeover ended without an outcome; cancelling the sign-in");
        if let Err(e) = cancel_attempt(
            &self.resources,
            caller.tenant_id,
            caller.user_id,
            caller.attempt.attempt_id(),
        )
        .await
        {
            warn!(user_id = %caller.user_id, error = %e, "Sciotte takeover cancel failed");
        }
    }
}

/// Relay both ways, the viewer's input served first, until one side ends.
async fn relay_until_end(viewer: &mut WebSocket, service: &mut ServiceWebSocket) -> Ended {
    let mut budget = InputBudget::new();
    loop {
        let ended = tokio::select! {
            biased;
            from_viewer = viewer.recv() => from_viewer_to_service(from_viewer, service, &mut budget).await,
            from_service = service.next() => from_service_to_viewer(from_service, viewer).await,
        };
        if let Some(ended) = ended {
            return ended;
        }
    }
}

/// Pass one viewer message on to the service; how the relay ended, if it did.
async fn from_viewer_to_service(
    message: Option<Result<Message, axum::Error>>,
    service: &mut ServiceWebSocket,
    budget: &mut InputBudget,
) -> Option<Ended> {
    match message {
        Some(Ok(Message::Text(text))) => {
            let input = admitted(&text, budget)?;
            service
                .send(WebSocketMessage::text(input))
                .await
                .is_err()
                .then_some(Ended::ServiceGone)
        }
        Some(Ok(Message::Close(_)) | Err(_)) | None => Some(Ended::ViewerLeft),
        Some(Ok(_)) => None,
    }
}

/// Pass one service message on to the viewer, keeping the outcome back for
/// the platform to conclude; how the relay ended, if it did.
async fn from_service_to_viewer<E>(
    message: Option<Result<WebSocketMessage, E>>,
    viewer: &mut WebSocket,
) -> Option<Ended> {
    match message {
        Some(Ok(WebSocketMessage::Binary(frame))) => viewer
            .send(Message::Binary(frame))
            .await
            .is_err()
            .then_some(Ended::ViewerLeft),
        Some(Ok(WebSocketMessage::Text(text))) => {
            match serde_json::from_str::<TakeoverEvent>(&text) {
                Ok(TakeoverEvent::Outcome { status_code, body }) => {
                    Some(Ended::Outcome { status_code, body })
                }
                Ok(_) => viewer
                    .send(Message::Text(text.as_str().into()))
                    .await
                    .is_err()
                    .then_some(Ended::ViewerLeft),
                Err(e) => {
                    debug!(error = %e, "Sciotte sent a takeover message this build does not read");
                    None
                }
            }
        }
        Some(Ok(WebSocketMessage::Close(_)) | Err(_)) | None => Some(Ended::ServiceGone),
        Some(Ok(_)) => None,
    }
}

/// `text` as the input sciotte reads, when it is one and the viewer's budget
/// has room for it; anything else is dropped here.
fn admitted(text: &str, budget: &mut InputBudget) -> Option<String> {
    if !budget.take() {
        return None;
    }
    let input = serde_json::from_str::<TakeoverInput>(text).ok()?;
    serde_json::to_string(&input).ok()
}

/// The answer a viewer gets when the takeover ended with no outcome: the
/// platform's own refusal, as an HTTP step would have answered it.
async fn takeover_ended() -> (u16, Value) {
    let mut ended = AppError::invalid_input("This sign-in check has ended. Try connecting again.");
    ended.details = Some(Box::new(
        serde_json::json!({ "reason": TAKEOVER_ENDED_REASON }),
    ));
    answer_of(ended.into_response()).await
}

/// A response's status and JSON body.
async fn answer_of(response: Response) -> (u16, Value) {
    let status = response.status();
    let body = to_bytes(response.into_body(), MAX_ANSWER_BYTES)
        .await
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    (status.as_u16(), body)
}

/// Send the sign-in's answer as the takeover's last event.
async fn send_answer(viewer: &mut WebSocket, status_code: u16, body: Value) {
    let event = TakeoverEvent::Outcome { status_code, body };
    if let Ok(text) = serde_json::to_string(&event) {
        if let Err(e) = viewer.send(Message::Text(text.into())).await {
            debug!(error = %e, "Takeover viewer left before its answer");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pierre_middleware::provider_link_token::{
        mint_takeover_link_token, verify_takeover_link_token, MintTakeoverTokenArgs, TakeoverLink,
    };

    use super::*;

    fn claims(link: Option<TakeoverLink>) -> TakeoverTokenClaims {
        let token = mint_takeover_link_token(
            &MintTakeoverTokenArgs {
                user_id: Uuid::from_u128(1),
                tenant_id: Uuid::from_u128(2),
                provider: "sciotte_garmin",
                attempt_id: "attempt-0001",
                link,
            },
            "secret",
        )
        .unwrap(); // Safe: test assertion
        verify_takeover_link_token(&token, "secret").unwrap() // Safe: test assertion
    }

    #[test]
    fn a_sign_in_from_a_channel_link_keeps_its_post_back_context() {
        let caller = Caller::of(claims(Some(TakeoverLink {
            channel: "slack".to_owned(),
            channel_thread: Some("T42".to_owned()),
            target: "garmin".to_owned(),
        })))
        .unwrap(); // Safe: test assertion
        let link = caller.link.expect("the channel context is carried");
        assert_eq!(link.channel, "slack", "the post-back goes to its channel");
        assert_eq!(
            link.channel_thread.as_deref(),
            Some("T42"),
            "the post-back goes to its thread"
        );
        assert_eq!(link.target, "garmin", "the post-back names its target");
        assert_eq!(
            caller.user_id,
            Uuid::from_u128(1),
            "the caller is the token's athlete"
        );
        assert_eq!(
            caller.tenant_id,
            Uuid::from_u128(2),
            "the caller is in the token's tenant"
        );
        assert_eq!(
            caller.attempt.attempt_id(),
            "attempt-0001",
            "the caller holds the token's attempt"
        );
    }

    #[test]
    fn an_in_app_sign_in_posts_back_nowhere() {
        assert!(
            Caller::of(claims(None)).unwrap().link.is_none(), // Safe: test assertion
            "an in-app sign-in carried a channel to post back to"
        );
    }

    #[test]
    fn a_viewer_sends_at_most_its_budget_a_second_and_only_takeover_input() {
        let mut budget = InputBudget::new();
        let text = r#"{"type":"text","text":"a"}"#;
        let admitted_count = (0..100)
            .filter(|_| admitted(text, &mut budget).is_some())
            .count();
        assert_eq!(
            admitted_count, MAX_INPUTS_PER_SECOND as usize,
            "a viewer sent more than its budget in one second"
        );
        let mut fresh = InputBudget::new();
        assert!(
            admitted(r#"{"type":"script"}"#, &mut fresh).is_none(),
            "a message that is not takeover input reached the service"
        );
    }

    /// A parity check between two artefacts nothing at runtime joins: this
    /// crate builds the takeover link, the web app routes its path and reads
    /// the `token` fragment key [`takeover_page_url`] writes.
    #[test]
    fn the_web_app_routes_the_takeover_path_and_reads_its_fragment() {
        let web = fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../frontend/src/utils/tokenLink.ts"
        ))
        .expect("the web app's link module");
        assert!(
            web.contains(&format!("export const TAKEOVER_PATH = '{TAKEOVER_PAGE}';")),
            "the web app must route {TAKEOVER_PAGE}"
        );
        assert!(
            web.contains(&format!(".get('{TAKEOVER_TOKEN_KEY}')")),
            "the web app must read the takeover token under the fragment key {TAKEOVER_TOKEN_KEY}"
        );
    }
}
