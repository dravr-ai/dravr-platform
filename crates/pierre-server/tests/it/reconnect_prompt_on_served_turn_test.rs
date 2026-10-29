// ABOUTME: A turn a sibling connection served must still hand the athlete a real reconnect link
// ABOUTME: Pins that on both loops, and that a reconnect owing a provider notice offers the hosted picker

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Serving a window without the elected provider must not cost the reconnect.
//!
//! When the elected connection's token is dead and a healthy sibling answers,
//! `get_activities` returns `Ok`, so `pending_provider_auth_required` is never
//! raised. Nothing in that `Ok` says a source is missing unless the tool's
//! `reconnect_required` sidecar is read — and without a reader the athlete gets
//! a partial window presented as a whole one: no control to press, and not even
//! a sentence naming what dropped.
//!
//! These tests pin the sidecar's two readers, end to end minus the model. The
//! tool stamps the dead BACKEND key into its result; `tool_results`' projection
//! carries the sidecar into the prompt, so the agent learns the window is short
//! a source before it writes a word; and the tool loop reads the same result as
//! the soft `served_without_provider` signal, which `auth_recovery` turns into a
//! real minted URL appended to the answer instead of replacing it. One test
//! drives the public `pierre_chat_pipeline::run` so the tail that carries the
//! offer out past post-processing is guarded as well as the stage that mints it.
//!
//! The blank path — nothing could serve — is pinned alongside, because "warn the
//! athlete and ask them to re-auth" must stay exactly as strong as it is, down
//! to the ordinary path a linkless failed mint falls through to.
//!
//! And the headless (Copilot ACP) path is pinned on its own, because it is the
//! one production resolves and the one that holds no tool payloads: its tools
//! run inside a subprocess that calls Dravr back over the loopback `/mcp`, so
//! there the offer travels a shared per-turn store instead of a return value.
//! Every reader above would be green with that path handing the athlete nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::{Transport, TransportPolicy};
use secrecy::SecretString;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use embacle::types::{RunnerError, ToolCallRequest};
use futures_util::stream;
use pierre_chat_pipeline::stages::auth_recovery::{
    apply_auth_recovery, AuthRecovery, AuthRecoveryDeps,
};
use pierre_chat_pipeline::{
    build_envelope, InputSource, PipelineHooks, QuotaState, ReplyBlock, SurfaceId, SurfaceProfile,
    SurfaceRequest, TurnInput, TurnOrigin, TurnState, TurnTelemetry,
};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    KEY_PROVIDER_REAUTH_SERVED_NO_LINK,
};
use pierre_core::constants::oauth::providers::provider_terms_version;
use pierre_core::errors::AppError;
use pierre_core::models::{
    ActivityBuilder, ConnectionType, ConversationRecord, ConversationTurnId, MessageRecord,
    SportType, TenantId, TenantOAuthCredentials, CHANNEL_TYPE_WEB,
};
use pierre_llm::{
    ChatProvider, ChatRequest, ChatResponse, ChatStream, CopilotHeadlessConfig,
    CopilotHeadlessRunner, FunctionResponse, HeadlessStreamEvent, HeadlessToolResponse,
    LlmCapabilities, LlmProvider, ObservedToolCall, StreamChunk, Tool,
};
use pierre_mcp_server::context::ServerContext;
use pierre_middleware::provider_link_token::{
    verify_link_token, CONNECT_PROVIDER, HOSTED_SIGNIN_PATH,
};
use pierre_providers::ai_scope;
use pierre_services::chat_stream::{
    TurnEvent, TurnEventSink, TOOL_STATUS_COMPLETED, TOOL_STATUS_IN_PROGRESS,
};
use pierre_tool_runtime::headless_stream::forward_headless_stream;
use pierre_tool_runtime::implementations::data::GetActivitiesTool;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::finalize_headless_turn;
use pierre_tool_runtime::tool_loop_io::{ToolLoopParams, ToolLoopResult};
use pierre_tool_runtime::tool_results::render_tool_payload_for_prompt;
use pierre_tool_runtime::tool_results::{extract_activity_list, reconnect_offer_in_responses};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::common::{
    create_test_server_resources, create_test_server_resources_with_chat_provider, create_test_user,
};

/// The agent's own answer over the sibling's data. Every assertion about what
/// survives compares against this exact string.
const AGENT_ANSWER: &str =
    "Ta sortie longue de 200 km domine ta semaine. On garde le tempo pour jeudi.";

/// The base URL the minted reconnect link must be built on.
const TEST_BASE_URL: &str = "https://api.test.dravr.ai";

/// A `ToolLoopResult` shaped the way a tool loop hands one to the recovery
/// stages: the agent's text, and whichever re-auth signal the turn raised.
fn loop_result(blank: Option<&str>, served: Option<&str>) -> ToolLoopResult {
    ToolLoopResult {
        content: AGENT_ANSWER.to_owned(),
        usage: None,
        finish_reason: None,
        activity_list: None,
        tool_calls_count: 1,
        tools_called: vec!["get_activities".to_owned()],
        pending_provider_auth_required: blank.map(str::to_owned),
        served_without_provider: served.map(str::to_owned),
        guardian_denied: None,
        guardian_confirm: None,
        capability_claim_unverified: false,
        provider_warnings: Vec::new(),
    }
}

fn turn_input(user_id: Uuid, tenant: TenantId) -> TurnInput {
    TurnInput {
        origin: TurnOrigin::Athlete,
        input_source: InputSource::Typed,
        conversation_id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        // The athlete alone with the agent — the standing every mint-path
        // assertion in this file describes; a shared room gets the linkless
        // sentence instead (capability_recovery_e2e_test pins that).
        is_direct_message: true,
        content: "Comment se passe ma semaine ?".to_owned(),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
        quote: None,
        quota: QuotaState::Ok,
        mentioned_agent: None,
        steps_after: None,
    }
}

fn web_profile() -> SurfaceProfile {
    SurfaceProfile::resolve(&SurfaceRequest {
        surface: SurfaceId::Web,
        locale: "fr".to_owned(),
        transport: None,
        prose_contract: None,
    })
}

/// Register a healthy Strava connection holding two real rides in the durable
/// cache, then a Garmin connection with no session at all — elected last, so it
/// is the primary the turn tries and fails to authenticate.
async fn athlete_with_a_dead_primary(resources: &Arc<ServerContext>) -> (Uuid, TenantId) {
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenants = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    let long_ride = ActivityBuilder::new(
        "strava-ride-1".to_owned(),
        "Sortie longue".to_owned(),
        SportType::Ride,
        Utc::now() - Duration::days(2),
        7_200,
        "strava".to_owned(),
    )
    .distance_meters(200_000.0)
    .build();
    let tempo_run = ActivityBuilder::new(
        "strava-run-1".to_owned(),
        "Tempo".to_owned(),
        SportType::Run,
        Utc::now() - Duration::days(4),
        3_600,
        "strava".to_owned(),
    )
    .distance_meters(14_000.0)
    .build();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant, "strava", &[long_ride, tempo_run])
        .await
        .unwrap();

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, "garmin", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    (user_id, tenant)
}

/// Run the recovery stage against a live registry, mint and short-link store.
async fn recover(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    result: &mut ToolLoopResult,
) -> AuthRecovery {
    let admin_jwt_secret: Arc<str> = Arc::from("test-admin-jwt-secret-for-reconnect-minting");
    let registry = Arc::new(MessagingStringsRegistry::new());
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let short_links = resources.common.repos.short_links.clone();
    let input = turn_input(user_id, tenant);
    apply_auth_recovery(
        AuthRecoveryDeps {
            admin_jwt_secret: &admin_jwt_secret,
            base_url: TEST_BASE_URL,
            messaging_strings_registry: &registry,
            tool_runtime: &runtime,
            short_links: &short_links,
        },
        &input,
        &web_profile(),
        result,
    )
    .await
}

// ============================================================================
// The served turn
// ============================================================================

/// The whole bridge: a served window names the dead BACKEND, the tool loop reads
/// that as its soft signal, and the athlete gets both the sibling's real rides
/// and a reconnect offer carrying a real URL.
#[tokio::test]
async fn a_served_turn_yields_the_siblings_data_and_a_clickable_reconnect_prompt() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let response = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await;
    let payload = response
        .structured_content
        .expect("tool result carries structured content");

    // Half one: the sibling's real rows, attributed to the connection that
    // produced them.
    let ids: Vec<&str> = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec!["strava-ride-1", "strava-run-1"],
        "the healthy connection's rides must reach the athlete, newest first"
    );
    assert_eq!(
        payload.get("provider").and_then(Value::as_str),
        Some("strava"),
        "the window is attributed to the connection that produced it"
    );

    // Half two: the sidecar names the dead provider twice — once for the model
    // in the athlete's words, once as the backend key the mint routes on.
    let caveat = payload
        .get("reconnect_required")
        .expect("the dead provider must still be surfaced for reconnection");
    assert_eq!(
        caveat.get("provider").and_then(Value::as_str),
        Some("garmin"),
        "the model is addressed with the brand name, not the backend key"
    );
    assert_eq!(
        caveat.get("provider_slug").and_then(Value::as_str),
        Some("sciotte_garmin"),
        "the chat pipeline mints on the backend key: garmin's hosted login and \
         an OAuth authorize round-trip are different routes"
    );

    // The tool loop's own reader, over the payload exactly as it reaches it.
    let responses = vec![FunctionResponse {
        name: "get_activities".to_owned(),
        response: payload.clone(),
    }];
    assert_eq!(
        reconnect_offer_in_responses(&responses).as_deref(),
        Some("sciotte_garmin"),
        "the served window must raise the soft signal the recovery stage reads"
    );

    // And the recovery stage turns that slug into an offer with a real link.
    let mut result = loop_result(None, Some("sciotte_garmin"));
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(
        !recovery.owns_reply,
        "a served turn's reply belongs to the coach; the stage only adds to it"
    );
    let prompt = recovery
        .prompt
        .expect("a served turn still offers the athlete a reconnect control");
    assert_eq!(
        prompt.provider, "sciotte_garmin",
        "the prompt names the dead connection"
    );
    assert_eq!(
        prompt.display_name, "Garmin",
        "the control is labelled with the brand the athlete knows"
    );
    assert!(
        prompt.url.starts_with(TEST_BASE_URL),
        "the reconnect URL must be a real minted link on this server, got: {}",
        prompt.url
    );
    assert!(
        prompt.text.contains(&prompt.url),
        "the sentence carries the same URL for a surface that only autolinks: {}",
        prompt.text
    );
}

/// The agent's answer is the athlete's data. It survives underneath the offer —
/// appending is the whole difference from the blank path.
#[tokio::test]
async fn the_agent_answer_survives_the_reconnect_offer_on_a_served_turn() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let mut result = loop_result(None, Some("sciotte_garmin"));
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(
        result.content.starts_with(AGENT_ANSWER),
        "the delivered reply must still open with the coach's answer over the \
         sibling's data, got: {}",
        result.content
    );
    assert!(
        result.content.contains("200 km"),
        "the sibling's numbers must not be discarded to say a connection dropped, \
         got: {}",
        result.content
    );
    let offer = recovery
        .prompt
        .expect("a minted offer accompanies the answer");
    // This surface draws the control, so the sentence under the answer names the
    // provider and the link travels in the control's own field. The offer is
    // still APPENDED — that is the whole difference from the blank path — it is
    // just the linkless copy that gets appended here.
    let appended = MessagingStringsRegistry::new().render(
        KEY_PROVIDER_REAUTH_SERVED_NO_LINK,
        "fr",
        &["Garmin"],
    );
    assert!(
        result.content.ends_with(&appended),
        "the offer is appended below the answer, not spliced into it, got: {}",
        result.content
    );
    assert!(
        !result.content.contains(&offer.url),
        "the control carries the link, so the prose must not repeat it, got: {}",
        result.content
    );
    assert!(
        offer.text.contains(&offer.url),
        "the control's own text still carries the link it renders, got: {}",
        offer.text
    );
    assert!(
        result.content.contains("Garmin"),
        "the athlete is told which connection to restore, got: {}",
        result.content
    );
}

/// Record the athlete's acceptance of WHOOP's owner-authorization notice,
/// which every account owes before a WHOOP mint: with it accepted, the mint
/// reaches the tenant's missing WHOOP credentials and fails on those.
async fn accept_whoop_notice(resources: &Arc<ServerContext>, user_id: Uuid) {
    resources
        .common
        .repos
        .users
        .record_provider_terms(user_id, "whoop", provider_terms_version("whoop").unwrap())
        .await
        .unwrap();
}

/// A mint that cannot produce a URL costs the control, never the answer.
#[tokio::test]
async fn a_failed_mint_on_a_served_turn_keeps_the_answer_and_drops_the_control() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    accept_whoop_notice(&resources, user_id).await;

    // WHOOP is an OAuth provider with no credentials configured for this
    // tenant, so `mint_oauth_authorize_url` refuses and there is no link.
    let mut result = loop_result(None, Some("whoop"));
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(
        recovery.prompt.is_none(),
        "with no URL there is nothing for a surface to draw a control around"
    );
    assert!(
        !recovery.owns_reply,
        "a served turn's reply still belongs to the coach when the mint fails"
    );
    assert!(
        result.content.starts_with(AGENT_ANSWER),
        "the answer must survive a failed mint, got: {}",
        result.content
    );
    assert!(
        result.content.contains("WHOOP"),
        "the athlete is still told which connection dropped, got: {}",
        result.content
    );
    assert!(
        !result.content.contains("http"),
        "no half-built link may reach the athlete, got: {}",
        result.content
    );
}

// ============================================================================
// The blank turn, unchanged
// ============================================================================

/// Nothing served the ask: the reply IS the reconnect message, exactly as
/// before. A single-provider athlete must not lose the deterministic reply.
#[tokio::test]
async fn a_sole_dead_connection_still_blanks_to_the_reconnect_message() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let mut result = loop_result(Some("sciotte_garmin"), None);
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(
        recovery.owns_reply,
        "the blank path's reply is platform text and owns the turn"
    );
    let prompt = recovery
        .prompt
        .expect("the blank path still mints a reconnect control");
    assert_eq!(prompt.provider, "sciotte_garmin");
    assert_eq!(prompt.display_name, "Garmin");

    let expected = MessagingStringsRegistry::new().render(
        KEY_PROVIDER_REAUTH_REQUIRED,
        "fr",
        &["Garmin", prompt.url.as_str()],
    );
    assert_eq!(
        result.content, expected,
        "the blanked reply must be the locale-resolved reconnect message alone"
    );
    assert!(
        !result.content.contains(AGENT_ANSWER),
        "nothing answered the ask, so no model text may survive: {}",
        result.content
    );
}

/// A TrainingPeaks session that must be (re)established — a dead one, or an
/// athlete who asked the agent to connect it — reaches the athlete as a
/// TrainingPeaks reconnect message whose link opens the TrainingPeaks login.
/// The display name and the token's target are the two places a Strava
/// default would hide; the rest of the reply reads the same either way.
#[tokio::test]
async fn a_trainingpeaks_reconnect_names_it_and_opens_its_login() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let mut result = loop_result(Some("sciotte_trainingpeaks"), None);
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    let prompt = recovery
        .prompt
        .expect("a TrainingPeaks reconnect mints a control");
    assert_eq!(prompt.provider, "sciotte_trainingpeaks");
    assert_eq!(prompt.display_name, "TrainingPeaks");
    assert_eq!(
        result.content,
        MessagingStringsRegistry::new().render(
            KEY_PROVIDER_REAUTH_REQUIRED,
            "fr",
            &["TrainingPeaks", prompt.url.as_str()],
        )
    );

    let code = prompt
        .url
        .rsplit("/r/")
        .next()
        .expect("the control carries a short link");
    let target = resources
        .common
        .repos
        .short_links
        .resolve_short_link(code)
        .await
        .expect("resolve query succeeds")
        .expect("the short code resolves");
    let token = target
        .split_once(&format!("{HOSTED_SIGNIN_PATH}?token="))
        .map_or_else(
            || panic!("the short link opens the hosted login: {target}"),
            |(_, token)| urlencoding::decode(token).unwrap().into_owned(),
        );
    let claims = verify_link_token(
        &token,
        "test-admin-jwt-secret-for-reconnect-minting",
        "sciotte",
    )
    .expect("the minted token verifies");
    assert_eq!(
        claims.tgt, "trainingpeaks",
        "the hosted login must open the TrainingPeaks scraper, not the Strava default"
    );
}

/// A turn carrying both signals did not answer what the athlete asked, so it
/// still blanks — the served path never weakens the blank one.
#[tokio::test]
async fn a_turn_carrying_both_signals_takes_the_blank_path() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let mut result = loop_result(Some("sciotte_garmin"), Some("whoop"));
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(recovery.owns_reply, "the hard signal wins");
    let prompt = recovery.prompt.expect("the blank path mints its control");
    assert_eq!(
        prompt.provider, "sciotte_garmin",
        "the provider that blanked the turn is the one to reconnect"
    );
    assert!(
        !result.content.contains(AGENT_ANSWER),
        "the blanked reply replaces the model's words: {}",
        result.content
    );
}

/// A blank turn whose mint fails hands the athlete a sentence and no link, and
/// that sentence is not a finished reply.
///
/// The short-circuit above stage 14a exists for the ONE shape it was built for:
/// a blanked turn carrying a minted link. A linkless fallback taking it too
/// would skip the identity re-ask and every post-processing stage, and leave the
/// envelope with no content blocks — so `owns_reply` stays false here and the
/// turn walks the ordinary path.
#[tokio::test]
async fn a_failed_mint_on_a_blank_turn_falls_through_to_post_processing() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    accept_whoop_notice(&resources, user_id).await;

    // WHOOP is an OAuth provider with no credentials configured for this
    // tenant, so `mint_oauth_authorize_url` refuses and there is no link.
    let mut result = loop_result(Some("whoop"), None);
    let recovery = recover(&resources, user_id, tenant, &mut result).await;

    assert!(
        !recovery.owns_reply,
        "a linkless reconnect sentence must fall through to the identity re-ask \
         and post-processing, exactly as the ordinary reply path does"
    );
    assert!(
        recovery.prompt.is_none(),
        "with no URL there is nothing for a surface to draw a control around"
    );

    let expected = MessagingStringsRegistry::new().render(
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "fr",
        &["WHOOP"],
    );
    assert_eq!(
        result.content, expected,
        "the blanked reply is the locale-resolved link-less copy alone"
    );
    assert!(
        !result.content.contains(AGENT_ANSWER),
        "nothing answered the ask, so no model text may survive: {}",
        result.content
    );
}

// ============================================================================
// The other reader: the model's own prompt
// ============================================================================

/// The agent has to LEARN that the window it is answering from is short a
/// source, or it presents a partial history as a complete one.
///
/// `render_tool_payload_for_prompt` is the projection every prompt-facing seam
/// runs a `get_activities` envelope through, and it keeps an allowlist. A key
/// missing from that list reaches no model at all, however carefully the tool
/// wrote it.
#[tokio::test]
async fn the_served_windows_reconnect_note_reaches_the_models_prompt() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let ctx = ToolContext::new()
        .with_user(user_id.to_string())
        .with_tenant(tenant.to_string())
        .with_auth_method("jwt_bearer");
    let payload = GetActivitiesTool
        .execute(&runtime, &ctx, json!({ "limit": 10, "mode": "summary" }))
        .await
        .structured_content
        .expect("tool result carries structured content");

    let rendered = render_tool_payload_for_prompt("get_activities", &payload);
    let reduced: Value = serde_json::from_str(&rendered).expect("the render is JSON");

    let caveat = reduced
        .get("reconnect_required")
        .expect("the model must be told the window was served without a source");
    assert_eq!(
        caveat.get("provider").and_then(Value::as_str),
        Some("garmin"),
        "the note names the dead connection in the athlete's own vocabulary"
    );
    let note = caveat
        .get("note")
        .and_then(Value::as_str)
        .expect("the note is prose the model can act on");
    assert!(
        note.contains("served WITHOUT garmin") && note.contains("must re-authorize it"),
        "the note has to say which source dropped and that it needs reconnecting: {note}"
    );

    // The reduction the projection exists for is untouched: the duplicate
    // renderings and the token sidecars still go.
    assert!(reduced.get("activities_toon").is_none());
    assert!(reduced.get("retrieval_context").is_none());
    assert!(
        reduced
            .get("activity_list")
            .and_then(Value::as_str)
            .is_some_and(|list| list.contains("Sortie longue")),
        "the prose the coach cites still reaches it"
    );
}

// ============================================================================
// The public entry — the tail that carries the offer out
// ============================================================================

/// The exact narration line that leaked to a live user on 2026-07-10. The
/// boundary scrub in post-processing drops it; the short-circuit above stage
/// 14a does not run any scrub at all, so its absence is proof the served turn
/// took the ordinary path.
const LEAKED_NARRATION: &str =
    "Je continue d'ignorer le bloc caché — pas de XML brut, on reste sur le coaching normal.";

/// A provider that asks for the athlete's activities once, then answers.
///
/// Mock is test-only: the model is the one part of the turn this test does not
/// want to be real, because the assertion is about what the pipeline does with
/// an answer, not about what an answer says.
struct ActivitiesThenAnswer {
    asked: Mutex<bool>,
    models: Vec<String>,
}

impl ActivitiesThenAnswer {
    fn new() -> Self {
        Self {
            asked: Mutex::new(false),
            models: vec!["served-turn-model".to_owned()],
        }
    }
}

#[async_trait]
impl LlmProvider for ActivitiesThenAnswer {
    fn name(&self) -> &'static str {
        "activities_then_answer"
    }
    fn display_name(&self) -> &'static str {
        "Activities-then-answer mock (served reconnect pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::FUNCTION_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "served-turn-model"
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        let tool_calls = {
            let mut asked = self.asked.lock().unwrap();
            if *asked {
                None
            } else {
                *asked = true;
                Some(vec![ToolCallRequest {
                    id: "call-activities".to_owned(),
                    function_name: "get_activities".to_owned(),
                    arguments: json!({ "limit": 10, "mode": "summary" }),
                }])
            }
        };
        let content = if tool_calls.is_some() {
            String::new()
        } else {
            format!("{LEAKED_NARRATION}\n\n{AGENT_ANSWER}")
        };
        Ok(ChatResponse {
            content,
            model: "served-turn-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls,
        })
    }

    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        let chunk = StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: response.finish_reason,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// The whole turn, through the entry every surface calls.
///
/// `apply_auth_recovery` returning a prompt is worth nothing if the pipeline
/// drops it on the floor after post-processing, and that tail is one line with
/// one caller. Driving `pierre_chat_pipeline::run` is what pins it: the athlete
/// gets the agent's answer AND a reconnect control carrying a link this server
/// really minted.
#[tokio::test]
async fn a_served_turn_carries_its_reconnect_control_out_of_the_public_entry() {
    let provider: Arc<dyn LlmProvider> = Arc::new(ActivitiesThenAnswer::new());
    let resources = create_test_server_resources_with_chat_provider(provider)
        .await
        .unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "served reconnect pin",
            "served-turn-model",
            None,
            None,
        )
        .await
        .unwrap();

    let mut input = turn_input(user_id, tenant);
    input.conversation_id = conversation.id.clone();

    let ctx = resources.chat_pipeline_context();
    // Over the web app its profile names, as `turn_service` declares it
    // (carnet#724).
    let envelope = ai_scope::serve_over(
        Transport::WebApp,
        pierre_chat_pipeline::run(&ctx, input, &web_profile(), &PipelineHooks::none()),
    )
    .await
    .expect("a window a sibling served must produce a served turn");

    let reconnect = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Reconnect {
                provider,
                display_name,
                url,
                text,
            } => Some((provider, display_name, url, text)),
            _ => None,
        })
        .expect("the served turn's envelope must carry the reconnect control");
    let (provider, display_name, url, text) = reconnect;

    assert_eq!(
        provider, "sciotte_garmin",
        "the control names the dead connection by the key the mint routes on"
    );
    assert_eq!(
        display_name, "Garmin",
        "the control is labelled with the brand the athlete knows"
    );
    assert!(
        text.contains(url.as_str()),
        "the field and the sentence must carry the same link: {text}"
    );

    // The URL is a link this server minted, not a placeholder: it resolves
    // through the short-link store to the athlete's hosted-login page.
    let prefix = format!("{}/r/", ctx.config.base_url.trim_end_matches('/'));
    let code = url
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("the control must carry a short link on {prefix}, got {url}"));
    let target = resources
        .common
        .repos
        .short_links
        .resolve_short_link(code)
        .await
        .unwrap()
        .expect("the minted code must resolve");
    assert!(
        target.contains(&format!("{HOSTED_SIGNIN_PATH}?token=")),
        "the short link must resolve to the hosted-login mint, got: {target}"
    );
    assert!(
        !target.to_lowercase().contains("sciotte"),
        "the hosted-login link names no backend: {target}"
    );

    // And the answer underneath it is the agent's own, post-processed rather
    // than handed out verbatim by the blank path's short circuit.
    let prose = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Prose { text } => Some(text),
            _ => None,
        })
        .expect("the coach's answer must still be delivered");
    assert!(
        prose.contains("200 km"),
        "the sibling's numbers must reach the athlete: {prose}"
    );
    assert!(
        !prose.contains("bloc caché"),
        "post-processing's narration scrub must have run over this reply, which \
         the blank path's short circuit skips entirely: {prose}"
    );
    assert!(
        !envelope.assistant.message.content.contains("bloc caché"),
        "the durable row is the post-processed reply too: {}",
        envelope.assistant.message.content
    );
}

// ============================================================================
// The headless (Copilot ACP) turn — where the athletes are
// ============================================================================

/// A provider advertising `SDK_TOOL_CALLING` routes its turn to the headless
/// loop, and Copilot ACP is what production resolves. That loop holds no tool
/// payloads at all: its tools run inside the ACP subprocess, which reaches Dravr
/// back over the loopback `/mcp` on per-request executors of its own. So the
/// served-without-a-provider offer has to travel out of that subprocess through
/// the shared per-turn store, exactly as a Guardian block does.
///
/// This drives that whole path with production code: the loopback dispatch that
/// stamps the offer, the turn assembly that drains it, the recovery stage that
/// mints on it, and the envelope the surface finally reads. What is NOT
/// production here is the ACP reply itself — the subprocess needs a live
/// `copilot` binary, and the assertion is about what the platform does with an
/// answer, not about what an answer says.
fn acp_reply() -> HeadlessToolResponse {
    HeadlessToolResponse {
        content: AGENT_ANSWER.to_owned(),
        model: HEADLESS_MODEL.to_owned(),
        tool_calls: vec![ObservedToolCall {
            id: "acp-tool-1".to_owned(),
            title: "get_activities".to_owned(),
            status: "Completed".to_owned(),
            ..ObservedToolCall::default()
        }],
        usage: None,
        finish_reason: Some("stop".to_owned()),
    }
}

/// The model the ACP turn ran on, asserted wherever the turn reports it.
const HEADLESS_MODEL: &str = "claude-opus-4";

/// Dispatch `get_activities` the way the ACP subprocess's `/mcp` loopback does:
/// through its own per-request executor, carrying this turn's ACP token. Returns
/// the payload the subprocess received.
async fn loopback_get_activities(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    acp_turn_token: &str,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    // The loopback serves the web turn that holds the token (carnet#724).
    let executor = UniversalExecutor::new(runtime)
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .with_turn_token(acp_turn_token.to_owned());
    let response = executor
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters: json!({ "limit": 10, "mode": "summary" }),
            user_id: user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(tenant.to_string()),
        })
        .await
        .expect("a window the healthy sibling can serve answers over the loopback");
    assert!(
        response.success,
        "the dead primary must not fail the dispatch: {:?}",
        response.error
    );
    response
        .result
        .expect("the loopback dispatch carries the tool's payload")
}

/// Finish the headless turn through the production assembly.
///
/// `finalize_headless_turn` is what every ACP turn returns through, and the ACP
/// runner it takes is only reached when the reply is degenerate — this one is
/// the agent's own answer, so nothing is spawned.
async fn finish_headless_turn(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    acp_turn_token: &str,
) -> ToolLoopResult {
    finish_headless_turn_with(
        resources,
        user_id,
        tenant,
        acp_turn_token,
        acp_reply(),
        Arc::new(ActivitiesThenAnswer::new()),
        None,
    )
    .await
}

/// A turn whose reply streamed to `sink` in `deltas` before it ended.
struct StreamedReply {
    sink: TurnEventSink,
    deltas: Vec<&'static str>,
}

/// [`finish_headless_turn`] for a given ACP `reply`, with `provider` as the
/// chain the turn runs on — the one the text loop asks when the reply hands the
/// turn to it — and, with `streamed`, on a turn that streamed its text: the
/// deltas cross the production forwarder first, which decides what reached
/// the sink, and the assembly is handed what it would be in production.
async fn finish_headless_turn_with(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    acp_turn_token: &str,
    reply: HeadlessToolResponse,
    provider: Arc<dyn LlmProvider>,
    streamed: Option<StreamedReply>,
) -> ToolLoopResult {
    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let executor = Arc::new(
        UniversalExecutor::new(runtime)
            .with_scopes(OAuthScope::self_grant())
            .with_transport(Transport::WebApp)
            .with_turn_token(acp_turn_token.to_owned()),
    );
    // The assembly reads the provider for its name alone unless the reply hands
    // the turn to the text loop; the ACP runner below is the one that served it.
    let provider = ChatProvider::Custom(provider);
    let runner = CopilotHeadlessRunner::with_config(CopilotHeadlessConfig::default());
    let tools = Tool {
        function_declarations: Vec::new(),
    };
    let request = ChatRequest::new(Vec::new());
    let (reply, stream_sink, prose_delivered) = match streamed {
        Some(StreamedReply { sink, deltas }) => {
            let events: Vec<Result<HeadlessStreamEvent, RunnerError>> = deltas
                .into_iter()
                .map(|delta| Ok(HeadlessStreamEvent::TextDelta(delta.to_owned())))
                .chain([Ok(HeadlessStreamEvent::Done(reply))])
                .collect();
            let turn = forward_headless_stream(Box::pin(tokio_stream::iter(events)), "", &sink)
                .await
                .expect("the stream ends on its Done event");
            (turn.response, Some(sink), turn.prose_delivered)
        }
        None => (reply, None, false),
    };
    let streamed_to = stream_sink.clone().filter(|_| prose_delivered);
    let user = user_id.to_string();
    let params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: HEADLESS_MODEL,
        user_id: &user,
        tenant_id: tenant,
        // Two rounds: the reply's own blocks, then the ask carrying their results.
        max_iterations: 2,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink,
        mcp_servers: Vec::new(),
    };
    finalize_headless_turn(
        reply,
        streamed_to.as_ref(),
        &runner,
        &request,
        &params,
        "prompt",
        &mut request.messages.clone(),
    )
    .await
    .expect("a non-degenerate ACP reply finishes without a retry")
}

/// A chain that answers every ask with the agent's reply.
///
/// Mock is test-only: what is under test is where the turn's loopback signals
/// go once the text loop has taken the turn, not what the model says.
struct AnswersFromResults {
    models: Vec<String>,
}

#[async_trait]
impl LlmProvider for AnswersFromResults {
    fn name(&self) -> &'static str {
        "answers_from_results"
    }
    fn display_name(&self) -> &'static str {
        "Answers-from-results mock (text-loop hand-off pin)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SDK_TOOL_CALLING | LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        HEADLESS_MODEL
    }
    fn available_models(&self) -> &[String] {
        &self.models
    }
    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        Ok(ChatResponse {
            content: AGENT_ANSWER.to_owned(),
            model: HEADLESS_MODEL.to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }
    async fn complete_stream(&self, request: &ChatRequest) -> Result<ChatStream, AppError> {
        let response = self.complete(request).await?;
        let chunk = StreamChunk {
            delta: response.content,
            is_final: true,
            finish_reason: response.finish_reason,
        };
        Ok(Box::pin(stream::once(async move { Ok(chunk) })))
    }
    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A Copilot SDK reply that asks for a tool as a text `<tool_call>` block hands
/// the turn to the text loop (carnet#1032) — and the loopback signal the turn
/// raised still reaches the athlete, though the headless assembly never ran.
///
/// The block calls a tool whose payload carries no offer, so the only way the
/// slug reaches the result is the shared store the loopback stamped.
#[tokio::test]
async fn a_headless_turn_handed_to_the_text_loop_keeps_its_loopback_offer() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    let acp_turn = Uuid::new_v4().to_string();
    loopback_get_activities(&resources, user_id, tenant, &acp_turn).await;

    let reply = HeadlessToolResponse {
        content:
            "<tool_call>\n{\"name\": \"get_connection_status\", \"arguments\": {}}\n</tool_call>"
                .to_owned(),
        model: HEADLESS_MODEL.to_owned(),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
    };
    let provider = Arc::new(AnswersFromResults {
        models: vec![HEADLESS_MODEL.to_owned()],
    });
    let result = finish_headless_turn_with(
        &resources, user_id, tenant, &acp_turn, reply, provider, None,
    )
    .await;

    assert_eq!(
        result.content, AGENT_ANSWER,
        "the text loop answered the turn from the block's result"
    );
    assert_eq!(
        result.tools_called,
        vec!["get_connection_status".to_owned()],
        "the reply's own block ran through the executor"
    );
    assert_eq!(
        result.served_without_provider.as_deref(),
        Some("sciotte_garmin"),
        "the offer the loopback raised survives the hand-off"
    );
}

/// The ACP reply that hands the turn to the text loop: one `<tool_call>`
/// block, after the words `preamble` puts before it.
fn tool_call_reply(preamble: &str) -> HeadlessToolResponse {
    HeadlessToolResponse {
        content: format!("{preamble}{CONNECTION_STATUS_BLOCK}"),
        model: HEADLESS_MODEL.to_owned(),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
    }
}

/// The block a reply asks for its connections with, as text.
const CONNECTION_STATUS_BLOCK: &str =
    "<tool_call>\n{\"name\": \"get_connection_status\", \"arguments\": {}}\n</tool_call>";

/// Run `reply` as a turn that streamed `deltas`, handed to the text loop, and
/// return the result with every event its sink received, in order.
async fn stream_turn_handed_to_the_text_loop(
    reply: HeadlessToolResponse,
    deltas: Vec<&'static str>,
) -> (ToolLoopResult, Vec<TurnEvent>) {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    let acp_turn = Uuid::new_v4().to_string();
    let provider = Arc::new(AnswersFromResults {
        models: vec![HEADLESS_MODEL.to_owned()],
    });
    let (sink, mut events) = mpsc::unbounded_channel();
    let result = finish_headless_turn_with(
        &resources,
        user_id,
        tenant,
        &acp_turn,
        reply,
        provider,
        Some(StreamedReply { sink, deltas }),
    )
    .await;
    let mut received = Vec::new();
    while let Ok(event) = events.try_recv() {
        received.push(event);
    }
    (result, received)
}

/// The `(title, status)` of every tool step in `events`, in order.
fn tool_steps(events: &[TurnEvent]) -> Vec<(&str, &str)> {
    events
        .iter()
        .filter_map(|event| match event {
            TurnEvent::Progress(progress) => {
                Some((progress.title.as_str(), progress.status.as_str()))
            }
            _ => None,
        })
        .collect()
}

/// A streamed Copilot reply handed to the text loop takes its words back
/// before the text loop runs the block: the reply is the text loop's answer,
/// and the words before the block would otherwise sit in the bubble while the
/// block's tool step ran under them.
#[tokio::test]
async fn a_headless_turn_handed_to_the_text_loop_takes_back_its_streamed_words_first() {
    let preamble = "Je regarde tes connexions. ";
    let (result, received) = stream_turn_handed_to_the_text_loop(
        tool_call_reply(preamble),
        vec![preamble, CONNECTION_STATUS_BLOCK],
    )
    .await;

    assert_eq!(
        result.content, AGENT_ANSWER,
        "the text loop answered the turn from the block's result"
    );
    assert!(
        matches!(
            received.as_slice(),
            [TurnEvent::ProseDelta(words), TurnEvent::ProseRetract, ..] if words == preamble
        ),
        "the words before the block reach the bubble, and are taken back before anything else \
         the turn sends: {received:?}"
    );
    let retractions = received
        .iter()
        .filter(|event| matches!(event, TurnEvent::ProseRetract))
        .count();
    assert_eq!(retractions, 1, "taken back once: {received:?}");
    assert_eq!(
        tool_steps(&received),
        vec![
            ("get_connection_status", TOOL_STATUS_IN_PROGRESS),
            ("get_connection_status", TOOL_STATUS_COMPLETED),
        ],
        "the block's call is a step on the trail, after the retraction"
    );
}

/// A streamed Copilot reply that wrote nothing but its `<tool_call>` block put
/// nothing in the bubble — the block never crosses the sink — so handing it to
/// the text loop takes nothing back: no retraction reaches the client.
#[tokio::test]
async fn a_headless_turn_that_streamed_only_its_block_sends_no_retraction() {
    let (result, received) =
        stream_turn_handed_to_the_text_loop(tool_call_reply(""), vec![CONNECTION_STATUS_BLOCK])
            .await;

    assert_eq!(
        result.content, AGENT_ANSWER,
        "the text loop answered the turn from the block's result"
    );
    assert!(
        !received
            .iter()
            .any(|event| matches!(event, TurnEvent::ProseDelta(_) | TurnEvent::ProseRetract)),
        "nothing reached the bubble, so nothing is taken back: {received:?}"
    );
    assert_eq!(
        tool_steps(&received),
        vec![
            ("get_connection_status", TOOL_STATUS_IN_PROGRESS),
            ("get_connection_status", TOOL_STATUS_COMPLETED),
        ],
        "the block's call is still a step on the trail"
    );
}

/// A headless reply handed to the text loop keeps the headless contract and
/// carries no activity list, though its block fetched the athlete's
/// activities: a surface without a list card would fold the list ahead of the
/// reply (carnet#1032).
#[tokio::test]
async fn a_headless_turn_handed_to_the_text_loop_folds_no_activity_list() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    let acp_turn = Uuid::new_v4().to_string();

    // The same call the block makes carries a list the text loop's extractor
    // captures, so an absent list below is the hand-off dropping it.
    let payload = loopback_get_activities(&resources, user_id, tenant, &acp_turn).await;
    assert!(
        extract_activity_list(&[FunctionResponse {
            name: "get_activities".to_owned(),
            response: payload,
        }])
        .is_some(),
        "get_activities serves a list the text loop would capture"
    );

    let reply = HeadlessToolResponse {
        content: "<tool_call>\n{\"name\": \"get_activities\", \"arguments\": {\"limit\": 10, \"mode\": \"summary\"}}\n</tool_call>"
            .to_owned(),
        model: HEADLESS_MODEL.to_owned(),
        tool_calls: Vec::new(),
        usage: None,
        finish_reason: Some("stop".to_owned()),
    };
    let provider = Arc::new(AnswersFromResults {
        models: vec![HEADLESS_MODEL.to_owned()],
    });
    let result = finish_headless_turn_with(
        &resources, user_id, tenant, &acp_turn, reply, provider, None,
    )
    .await;

    assert_eq!(result.content, AGENT_ANSWER);
    assert_eq!(
        result.tools_called,
        vec!["get_activities".to_owned()],
        "the reply's own block fetched the activities"
    );
    assert!(
        result.activity_list.is_none(),
        "a handed-off headless turn folds no list ahead of its reply"
    );
}

fn stored_message(id: &str, role: &str, content: &str) -> MessageRecord {
    MessageRecord {
        id: id.to_owned(),
        conversation_id: "conv-headless".to_owned(),
        role: role.to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: Some(HEADLESS_MODEL.to_owned()),
        finish_reason: Some("stop".to_owned()),
        content_blocks: None,
        created_at: "2026-08-29T00:00:00Z".to_owned(),
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: None,
    }
}

/// The turn as the envelope builder receives it, once every stage has run.
fn envelope_state(result: &ToolLoopResult, recovery: AuthRecovery) -> TurnState {
    TurnState {
        turn_id: ConversationTurnId::new(),
        user_message: stored_message("msg-user", "user", "Comment se passe ma semaine ?"),
        assistant_message: stored_message("msg-asst", "assistant", &result.content),
        conversation: ConversationRecord {
            id: "conv-headless".to_owned(),
            user_id: "user-headless".to_owned(),
            tenant_id: "tenant-headless".to_owned(),
            title: "Semaine".to_owned(),
            model: HEADLESS_MODEL.to_owned(),
            agent_id: None,
            session_id: None,
            total_tokens: 0,
            created_at: "2026-08-29T00:00:00Z".to_owned(),
            updated_at: "2026-08-29T00:00:00Z".to_owned(),
            group_id: None,
            channel_type: CHANNEL_TYPE_WEB.to_owned(),
            onboarding_state: None,
            archived_at: None,
            personal: false,
            subject_user_id: None,
            subject_group_id: None,
        },
        content: result.content.clone(),
        finish_reason: result.finish_reason.clone(),
        activity_list: result.activity_list.clone(),
        introduction_end: None,
        telemetry: TurnTelemetry {
            model: HEADLESS_MODEL.to_owned(),
            provider_name: "copilot_headless".to_owned(),
            tools_called: result.tools_called.clone(),
            tool_calls_count: result.tool_calls_count,
            activity_list_captured: result.activity_list.is_some(),
            activities_prefetched: false,
            usage: None,
            identity_leak: None,
            provider_warnings: Vec::new(),
        },
        quota: QuotaState::Ok,
        reconnect: recovery.prompt,
        verdict_chips: Vec::new(),
        scene_images: Vec::new(),
        actions: Vec::new(),
        actions_title: None,
        // The conversation above is bound to no agent.
        answered_by: None,
    }
}

/// The payoff, on the path production actually runs.
///
/// A Copilot ACP turn whose loopback served the window without Garmin must hand
/// the athlete a real reconnect control, not just whatever sentence the agent
/// chose to write — and must still hand them the sibling's numbers underneath it.
#[tokio::test]
async fn a_headless_turn_carries_the_reconnect_control_its_loopback_raised() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    let acp_turn = Uuid::new_v4().to_string();

    // The subprocess's tool call: the sibling's real rides answer the window,
    // and the payload names the connection that could not.
    let payload = loopback_get_activities(&resources, user_id, tenant, &acp_turn).await;
    let ids: Vec<&str> = payload
        .get("activities")
        .and_then(Value::as_array)
        .expect("activities array present")
        .iter()
        .filter_map(|a| a.get("id").and_then(Value::as_str))
        .collect();
    assert_eq!(
        ids,
        vec!["strava-ride-1", "strava-run-1"],
        "the healthy connection's rides must reach the ACP subprocess"
    );
    assert_eq!(
        payload
            .get("reconnect_required")
            .and_then(|c| c.get("provider_slug"))
            .and_then(Value::as_str),
        Some("sciotte_garmin"),
        "the served window names the dead backend for the mint to route on"
    );

    // The turn finishing. The loop saw none of that payload — the slug has to
    // come out of the per-turn store the dispatch stamped.
    let mut result = finish_headless_turn(&resources, user_id, tenant, &acp_turn).await;
    assert_eq!(
        result.served_without_provider.as_deref(),
        Some("sciotte_garmin"),
        "a headless turn must carry the soft signal its loopback raised"
    );
    assert_eq!(
        result.content, AGENT_ANSWER,
        "the ACP answer is delivered as written"
    );
    assert!(
        result.pending_provider_auth_required.is_none(),
        "nothing blanked: a served window never takes the hard path"
    );

    // The recovery stage mints on that slug, and the envelope draws a control.
    let recovery = recover(&resources, user_id, tenant, &mut result).await;
    assert!(
        !recovery.owns_reply,
        "a served turn's reply belongs to the coach; the stage only adds to it"
    );
    let envelope = build_envelope(&web_profile(), envelope_state(&result, recovery));

    let (provider, display_name, url, text) = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Reconnect {
                provider,
                display_name,
                url,
                text,
            } => Some((provider, display_name, url, text)),
            _ => None,
        })
        .expect("the headless turn's envelope must carry the reconnect control");
    assert_eq!(
        provider, "sciotte_garmin",
        "the control names the dead connection by the key the mint routes on"
    );
    assert_eq!(
        display_name, "Garmin",
        "the control is labelled with the brand the athlete knows"
    );
    assert!(
        url.starts_with(&format!("{TEST_BASE_URL}/r/")),
        "the control must carry a short link this server minted, got: {url}"
    );
    assert!(
        text.contains(url.as_str()),
        "the field and the sentence must carry the same link: {text}"
    );

    // And the agent's answer over the sibling's data survives underneath it.
    let prose = envelope
        .assistant
        .blocks
        .iter()
        .find_map(|block| match block {
            ReplyBlock::Prose { text } => Some(text),
            _ => None,
        })
        .expect("the coach's answer must still be delivered");
    assert!(
        prose.starts_with(AGENT_ANSWER),
        "the offer is added to the answer, not substituted for it: {prose}"
    );
    assert!(
        prose.contains("200 km"),
        "the sibling's numbers must reach the athlete: {prose}"
    );
    assert!(
        !prose.contains(url.as_str()),
        "on a surface that draws the control the URL stays out of the prose: {prose}"
    );
}

/// The store is a per-turn channel, not a standing flag.
///
/// An offer left behind would ride the athlete's NEXT headless turn — one with a
/// healthy window — and ask them to reconnect something that is already
/// connected. So the turn that raises it is the turn that consumes it.
#[tokio::test]
async fn a_headless_turn_drains_the_offer_it_consumed() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, tenant) = athlete_with_a_dead_primary(&resources).await;
    let acp_turn = Uuid::new_v4().to_string();

    loopback_get_activities(&resources, user_id, tenant, &acp_turn).await;
    let first = finish_headless_turn(&resources, user_id, tenant, &acp_turn).await;
    assert_eq!(
        first.served_without_provider.as_deref(),
        Some("sciotte_garmin"),
        "the turn that raised the offer carries it"
    );

    // A second turn for the same athlete, whose loopback called nothing.
    let next_turn = Uuid::new_v4().to_string();
    let second = finish_headless_turn(&resources, user_id, tenant, &next_turn).await;
    assert_eq!(
        second.served_without_provider, None,
        "a turn that served no window must offer nothing to reconnect"
    );
    assert_eq!(
        second.content, AGENT_ANSWER,
        "and answers exactly as it was written"
    );
}

// ============================================================================
// A reconnect that owes the provider's notice
// ============================================================================

/// A WHOOP reconnect for an account that owes WHOOP's owner authorization
/// hands out no WHOOP authorization URL: a chat link cannot carry the
/// acceptance, so the offer is the hosted connect picker, which shows the
/// notice with its box. Once the account has accepted it, the offer is WHOOP's
/// own authorization page again.
#[tokio::test]
async fn a_whoop_reconnect_owing_the_owner_authorization_offers_the_hosted_picker() {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let repos = &resources.common.repos;
    let tenant = repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap()
        .first()
        .expect("user has a tenant")
        .id;
    repos
        .tenants
        .store_oauth_credentials(&TenantOAuthCredentials {
            tenant_id: tenant,
            provider: "whoop".to_owned(),
            client_id: "whoop-reconnect-test-client".to_owned(),
            client_secret: SecretString::from("whoop-reconnect-test-secret"),
            redirect_uri: "http://localhost/api/oauth/callback/whoop".to_owned(),
            scopes: vec!["read:recovery".to_owned()],
            rate_limit_per_day: 1000,
        })
        .await
        .unwrap();

    let mut result = loop_result(Some("whoop"), None);
    let prompt = recover(&resources, user_id, tenant, &mut result)
        .await
        .prompt
        .expect("the athlete still gets a reconnect control");
    let code = prompt
        .url
        .strip_prefix(&format!("{TEST_BASE_URL}/r/"))
        .unwrap_or_else(|| panic!("the offer is a short link on this server: {}", prompt.url));
    let target = repos
        .short_links
        .resolve_short_link(code)
        .await
        .unwrap()
        .expect("the short link resolves");
    let token = target
        .strip_prefix(&format!("{TEST_BASE_URL}/providers/connect?token="))
        .unwrap_or_else(|| panic!("the offer opens the hosted connect picker: {target}"));
    let token = urlencoding::decode(token).unwrap();
    let claims = verify_link_token(
        &token,
        "test-admin-jwt-secret-for-reconnect-minting",
        CONNECT_PROVIDER,
    )
    .expect("a connect-scoped token for the picker");
    assert_eq!(claims.sub, user_id.to_string());

    repos
        .users
        .record_provider_terms(user_id, "whoop", provider_terms_version("whoop").unwrap())
        .await
        .unwrap();
    let mut result = loop_result(Some("whoop"), None);
    let prompt = recover(&resources, user_id, tenant, &mut result)
        .await
        .prompt
        .expect("a reconnect control");
    assert!(
        prompt
            .url
            .starts_with("https://api.prod.whoop.com/oauth/oauth2/auth"),
        "an account that accepted reconnects on WHOOP's page: {}",
        prompt.url
    );
}
