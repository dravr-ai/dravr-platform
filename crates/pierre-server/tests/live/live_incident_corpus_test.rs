// ABOUTME: The live-model eval lane — every 2026-08 coaching incident replayed against a REAL model over the production transport
// ABOUTME: Grades the DELIVERED body, not the stored row; a live-e2e target that fails, never skips, without its model
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live-model incident corpus.
//!
//! Action #1 of the 2026-08-23 post-mortem *Why E2E Kept Missing the Agent
//! Regressions*, which asked why a 325-binary suite with e2e coverage let four
//! consecutive days of live coaching failures through. The answer it reached:
//!
//! > Every e2e drives a scripted mock model. They verify the *pipeline's
//! > reaction* to known model behavior. But every failure originated in real
//! > model behavior no author scripted.
//!
//! A mock cannot regress the way a model does. This lane is the tier that can:
//! it drives a **real** model through production's transport — the pinned
//! Copilot CLI over embacle ACP — and the **real** chat pipeline, over the
//! **real** prompt corpus, and grades what came out.
//!
//! Which model answers is the environment's choice. The nightly lane points the
//! Copilot CLI at a model served on the runner (`COPILOT_PROVIDER_BASE_URL`),
//! because a corpus this size on a hosted account spends the quota athletes are
//! served from; its findings are therefore about the pipeline and transport
//! under an unscripted model, not about how the deployed model coaches.
//!
//! ## Three things it does differently
//!
//! 1. **Real providers.** The provider is built by
//!    [`ChatProvider::from_env`] — the same call `pierre-mcp-server.rs` makes —
//!    and injected through [`create_test_server_resources_with_chat_provider`],
//!    which wires `chat_provider` and leaves `llm_provider` `None` exactly as
//!    production does. The other helper wires it the opposite way, and pipeline
//!    code that reads `ctx.llm_provider` is then dead in production while green
//!    in tests; that is how the bounded identity re-ask shipped inert.
//!
//! 2. **The delivered body, not the persisted row.** Every assertion reads
//!    `messaging_messages.direction = 'outbound'`. Post-process stages rewrite
//!    the durable copy, so a persisted-row assertion passes while the athlete is
//!    receiving raw scaffolding — the 2026-08-18 lesson, and the shape of the
//!    08-23 chart drop.
//!
//! 3. **A messy fixture athlete.** Seeded fixtures were single-provider,
//!    single-identity and polite, so incident #3 (a 200 km ride described as a
//!    distance-less "WHOOP run") was invisible *by construction*. This lane's
//!    athlete carries Strava + WHOOP twins of one session, a distance-less
//!    sensor record, and a roster holding both "Phil" and "Philippe Tremblay".
//!
//! ## Running it
//!
//! A live test: every turn is a real model call and takes minutes, so it is
//! built only with the `live-e2e` feature — a plain `cargo test` never
//! compiles it and never counts it as passed. Built, it never skips
//! (carnet#805): with no provider env, no model id, or a model that never
//! answers, it fails. The guards that need no model — the fixture's ground
//! truth, the corpus order, the reproduce rule — run in the default suite as
//! `live_incident_eval_test`. The `Eval: Live Incident Corpus` workflow
//! provisions the model; a local run needs the pinned Copilot CLI on `PATH`
//! (`COPILOT_CLI_VERSION` in `docker/images/server/Dockerfile`), an Ollama
//! serving the model with `OLLAMA_CONTEXT_LENGTH=32768` (its 4096 default
//! truncates the prompt without an error), and the provider selection env the
//! server itself reads.
//!
//! ```bash
//! PIERRE_LLM_PROVIDER=copilot_headless \
//!   PIERRE_LLM_MODEL=qwen2.5:7b-instruct-q4_K_M PIERRE_LLM_RUNTIME_FALLBACK=false \
//!   COPILOT_OFFLINE=true COPILOT_PROVIDER_BASE_URL=http://localhost:11434/v1 \
//!   EMBACLE_ACP_PROMPT_TIMEOUT_SECS=1200 EMBACLE_ACP_MESSAGE_TIMEOUT_SECS=900 \
//!   cargo test --features live-e2e --test live_incident_corpus_test -- --nocapture
//! ```
//!
//! The ACP runner pins its model in `$HOME/.copilot/settings.json`; give a
//! local run its own `HOME` if that file is one you use.
//!
//! ## Reading a failure
//!
//! A **finding** is a statement about the model: the corpus reproduced a
//! regression. An **infra error** is the absence of any observation — the CLI
//! never started, the key was rejected, the turn timed out. A
//! **platform-answered** turn is a third thing: the pipeline completed, but a
//! deterministic branch wrote the reply — a reconnect sentence, a guardian
//! block — and no model spoke. All three are reported apart and only findings
//! gate the lane — a run that observed fewer than half its turns fails as an
//! infrastructure failure instead of passing on silence — because collapsing them reports a crashed subprocess or our
//! own localized copy as "the coach didn't draw the chart", which is how the
//! 2026-07 AMX segfaults masqueraded as quality regressions for a week.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(not(feature = "client-messaging"))]
compile_error!(
    "live_incident_corpus_test drives the Slack messaging pipeline and needs the \
     client-messaging feature; built without it the target would hold no test and pass"
);

#[path = "../common.rs"]
mod common;
#[path = "../helpers/mod.rs"]
mod helpers;
#[path = "../helpers/live_incident_corpus.rs"]
mod live_incident_corpus;

use crate::common::{create_test_server_resources_with_real_chat_provider, PLACEHOLDER_LLM_MODEL};
use crate::helpers::axum_test::AxumTestRequest;
use crate::live_incident_corpus::{
    athlete_run_metres, classify_findings, days_since_sunday, ground_truth, seed_fixture,
    Classified, Expect, Finding, Fixture, ATHLETE_RUN_DAYS, CORPUS, FIXTURE_WEEKS, SIGNING_SECRET,
    SUNDAY_RIDE_HOURS, SUNDAY_RIDE_KM, UNIVERSAL,
};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use pierre_core::models::{TenantId, WITHHELD_REPLY_FINISH_REASON};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_llm::judge::{ask_for_json, judge_request};
use pierre_llm::{ChatProvider, LlmProvider};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::messaging::MessagingRoutes;
use serde::Deserialize;
use serde_json::json;
use serial_test::serial;
use sha2::Sha256;
use std::env;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::time::sleep;

/// How long one turn may take before it is recorded as an infra error.
///
/// A Copilot Autopilot turn runs the whole tool loop and synthesis inside
/// one ACP prompt, which the runner caps at `EMBACLE_ACP_PROMPT_TIMEOUT_SECS`
/// (300s unset, which is what production runs). One margin past that so a
/// turn the server itself would have abandoned is attributed to the provider
/// rather than to this lane's patience — and read from the same variable, so
/// a lane that gives a CPU-served model a longer prompt cap does not then
/// cut the turn off at the hosted model's.
fn turn_timeout() -> Duration {
    const PRODUCTION_PROMPT_CAP_SECS: u64 = 300;
    const MARGIN_SECS: u64 = 30;
    let cap = env::var("EMBACLE_ACP_PROMPT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|secs| *secs > 0)
        .unwrap_or(PRODUCTION_PROMPT_CAP_SECS);
    Duration::from_secs(cap.saturating_add(MARGIN_SECS))
}

/// Poll interval while waiting for the delivered message to land.
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// How much of a delivered reply the failure report quotes. Enough to see
/// what the athlete actually got; short enough that eleven of them stay
/// readable in a CI log.
const DELIVERED_EXCERPT_CHARS: usize = 400;

/// A reply cut to [`DELIVERED_EXCERPT_CHARS`], the one length every printed
/// body in this lane is quoted at.
///
/// Counted in `char`s rather than bytes because an agent reply is French and
/// a byte slice would split an accent mid-codepoint.
fn excerpt(body: &str) -> String {
    body.chars().take(DELIVERED_EXCERPT_CHARS).collect()
}

/// Episode names to run, from `LIVE_INCIDENT_EVAL_EPISODES` (comma-separated).
///
/// Empty by default, which runs the whole corpus. Exists because probing one
/// episode otherwise costs the entire 11-turn corpus times three passes of
/// metered live calls, which is why this lane gets reasoned about from a
/// single nightly sample instead of being re-run.
///
/// NOT behaviour-neutral, and the caller has to know it: episodes share one
/// channel per surface with no conversation reset between them, so running
/// one in isolation strips the history the preceding episodes would have
/// left in its prompt — and the prompt is usually the thing under
/// investigation. Use it to iterate, never to produce a number that gets
/// compared against a full run.
fn episode_filter() -> Vec<String> {
    env::var("LIVE_INCIDENT_EVAL_EPISODES")
        .ok()
        .map(|v| {
            v.split(',')
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// How many times the whole corpus is driven, from
/// `LIVE_INCIDENT_EVAL_ATTEMPTS` (default 3, floor 1).
///
/// A live model samples, so a single pass cannot tell a defect from a draw.
/// Across four runs on 2026-08-26 the finding count moved 6 → 2 → 8 → 6 and
/// which turn produced the empty reply moved with it, while the missing
/// chart block failed all four. Driving the corpus more than once is what
/// lets the lane tell those two apart without anyone hand-picking which
/// failures to believe.
fn attempts() -> usize {
    env::var("LIVE_INCIDENT_EVAL_ATTEMPTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .max(1)
}

/// Seconds to wait between turns, from `LIVE_INCIDENT_EVAL_TURN_DELAY_SECS`.
///
/// Zero by default, because pacing is a property of the key in use rather
/// than of the corpus. A Cohere **trial** key allows 20 calls/minute and one
/// turn spends several (the agent-proposal re-rank, the turn itself, the
/// judge), so a local run on one wants roughly 10; a production key and the
/// Copilot primary want none. Surfaced as a knob instead of a baked-in sleep
/// so the lane never quietly pays for a limit the runner does not have.
fn turn_delay() -> Duration {
    Duration::from_secs(
        env::var("LIVE_INCIDENT_EVAL_TURN_DELAY_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0),
    )
}

/// The absence of any observation. Never a quality signal.
#[derive(Debug)]
struct InfraError {
    episode: &'static str,
    turn_index: usize,
    detail: String,
}

/// A turn the platform answered on its own: the pipeline ran to completion
/// and persisted a row, but its text is deterministic localized copy.
///
/// Neither a finding nor an infra error. The turn is reported and left
/// ungraded, and it counts toward no observation of the model — so a run
/// where the platform answered everything trips the observation floor
/// instead of reporting a healthy zero.
#[derive(Debug)]
struct PlatformAnswer {
    episode: &'static str,
    turn_index: usize,
    finish_reason: String,
    body: String,
}

/// What the judge returns.
#[derive(Deserialize)]
struct Verdict {
    /// `true` when the reply satisfies the question asked of it.
    holds: bool,
    /// One sentence of justification, surfaced in the failure report.
    #[serde(default)]
    rationale: String,
}

fn slack_sig(secret: &str, timestamp: &str, body: &str) -> String {
    let basestring = format!("v0:{timestamp}:{body}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(basestring.as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

/// A sciotte stand-in that serves the fixture's OWN activities.
///
/// The live fetch path returns its result directly, so whatever this serves
/// IS the athlete's history for any turn that resolves sciotte. Serving
/// anything other than what the fixture seeds gives the agent two
/// disagreeing accounts of one athlete and then grades it for noticing.
async fn spawn_eval_scraper() -> String {
    let sunday = (Utc::now() - ChronoDuration::days(days_since_sunday()))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let mut activities = vec![json!({
        "id": "eval-sunday-strava",
        "name": "Sortie longue",
        "sport_type": "ride",
        "start_date": sunday,
        "duration_seconds": SUNDAY_RIDE_HOURS * 3_600,
        "provider": "strava",
        "distance_meters": SUNDAY_RIDE_KM * 1_000.0,
    })];
    let sunday_offset = days_since_sunday();
    for week in 0..FIXTURE_WEEKS {
        for day in ATHLETE_RUN_DAYS {
            let offset = week * 7 + day;
            if offset % 7 == sunday_offset % 7 {
                continue;
            }
            activities.push(json!({
                "id": format!("eval-run-{week}-{day}"),
                "name": "Course",
                "sport_type": "run",
                "start_date": (Utc::now() - ChronoDuration::days(offset))
                    .format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                "duration_seconds": 3_600 + (day as u64 * 600),
                "provider": "strava",
                "distance_meters": athlete_run_metres(day),
            }));
        }
    }
    let count = activities.len();
    let payload = json!({ "count": count, "activities": activities, "head_complete": true });

    let app = Router::new()
        .route(
            "/auth/import-session",
            post(|| async { Json(json!({ "session_id": "cap-verified-session" })) }),
        )
        .route(
            "/api/athlete",
            get(|| async { Json(json!({ "display_name": "JF" })) }),
        )
        .route(
            "/api/activities",
            get(move || {
                let payload = payload.clone();
                async move { Json(payload) }
            }),
        );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

// -----------------------------------------------------------------------
// Driving a turn
// -----------------------------------------------------------------------

/// What actually reached the athlete.
struct Delivered {
    body: String,
    /// The `content_blocks` rail of the assistant row this turn produced.
    /// A chart lives here, not in the prose.
    blocks: Option<String>,
}

/// Who wrote the reply the athlete read.
enum Answered {
    /// A model produced it. The only shape the corpus grades.
    Model(Delivered),
    /// A deterministic branch of the pipeline produced it, and the assistant
    /// row carries that branch's `finish_reason` stamp. The sentence is
    /// localized copy from the messaging registry — no model wrote a word of
    /// it — so a judged expectation would be grading our own string as the
    /// agent's answer.
    Platform { finish_reason: String, body: String },
}

/// `finish_reason` stamps that mean a deterministic platform branch wrote
/// the reply rather than a model.
///
/// Structural in the same spirit as [`assistant_row_count`], and for the
/// same reason: the stamp is set at write time by the branch that
/// short-circuited, so it survives a reworded string and a sixth locale,
/// while the lane never matches on error prose.
///
/// The membership test is what the tool loop hands back, not the wording:
/// each of `pierre-tool-runtime`'s short-circuits returns
/// `content: String::new()`, so the sentence the athlete reads is whatever
/// the pipeline renders for an empty reply — a reconnect offer, the
/// localized guardian refusal, «je n'ai pas réussi à formuler une réponse».
/// [`WITHHELD_REPLY_FINISH_REASON`] is the mirror case: a model DID write,
/// and the platform replaced the words with its own apology.
///
/// `capability_claim_unverified` is deliberately absent. The
/// capability-recovery stage also sets it on rows whose MODEL text survived
/// verification unproven, so treating that stamp as platform-authored would
/// drop real model replies out of the corpus.
const PLATFORM_AUTHORED_FINISH_REASONS: &[&str] = &[
    "provider_auth_required",
    "guardian_denied",
    "guardian_confirm",
    "guardian_plan_rejected",
    "max_iterations",
    WITHHELD_REPLY_FINISH_REASON,
];

/// Did the model actually answer this turn?
///
/// A dispatch failure — dead provider, wrong model id, exhausted rate limit,
/// quota refusal — still delivers something: `report_dispatch_failure` and
/// `send_quota_denial_reply` send a canned localized string, so an outbound
/// row appears and the turn *looks* answered. Grading that string reports
/// «Dravr est temporairement indisponible» as "the coach didn't draw the
/// chart", which is a statement about our infrastructure wearing a
/// statement about the model's face. That mask cost the 2026-07 chat-eval a
/// week of chasing a segfault as a tool-discipline regression, and this lane
/// reproduced it on its first live run.
///
/// The discriminator is structural rather than a list of known error
/// strings: only a completed turn persists an assistant `chat_messages`
/// row. Both failure paths return before persistence, so an outbound
/// message with no new assistant row behind it is infrastructure by
/// construction — and it stays correct when the copy is reworded or a new
/// failure path is added.
///
/// What it answers is "did the pipeline complete?", which is narrower than
/// "did a model write this?". A deterministic branch that assigns the reply
/// and falls through post-process — the reconnect sentence, a guardian block
/// — persists a row like any completed turn.
/// [`PLATFORM_AUTHORED_FINISH_REASONS`] separates those.
async fn assistant_row_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
    count_for_tenant(
        &resources.agent.database,
        "SELECT COUNT(*) FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         WHERE c.tenant_id = $1 AND m.role = 'assistant'",
        tenant,
    )
    .await
}

/// `sql` is a `COUNT(*)` with the tenant id bound as `$1`, run on
/// whichever backend the test database is.
async fn count_for_tenant(db: &Database, sql: &str, tenant: TenantId) -> i64 {
    let tenant = tenant.to_string();
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_scalar(sql)
            .bind(&tenant)
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query_scalar(sql)
            .bind(&tenant)
            .fetch_one(pg.pool())
            .await
            .unwrap(),
    }
}

/// The newest row `sql` selects for the tenant bound as `$1`, on whichever
/// backend the test database is; `sql` selects two nullable text columns.
async fn latest_for_tenant(
    db: &Database,
    sql: &str,
    tenant: TenantId,
) -> Option<(Option<String>, Option<String>)> {
    let tenant = tenant.to_string();
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_as(sql)
            .bind(&tenant)
            .fetch_optional(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query_as(sql)
            .bind(&tenant)
            .fetch_optional(pg.pool())
            .await
            .unwrap(),
    }
}

/// Post one inbound Slack message and wait for the delivered reply.
///
/// Three outcomes, because the athlete reading a fluent French paragraph
/// does not tell you which of them produced it:
///
/// - `Err` — nothing was delivered inside [`turn_timeout`], or the turn
///   never persisted an assistant row. An infra error, not a finding: no
///   reply is not a bad reply.
/// - [`Answered::Platform`] — a deterministic branch answered and stamped
///   the row. Neither a finding nor an infra error: the pipeline worked, the
///   model simply never spoke.
/// - [`Answered::Model`] — a model's own text. The corpus grades this.
async fn drive_turn(
    resources: &Arc<ServerContext>,
    fixture: &Fixture,
    channel: &str,
    text: &str,
    baseline: i64,
    assistant_baseline: i64,
) -> Result<Answered, String> {
    let body = json!({
        "type": "event_callback",
        "event": {
            "type": "message",
            "user": "U_EVAL_JF",
            "text": text,
            "channel": channel,
            "ts": format!("{}.000001", Utc::now().timestamp()),
        }
    })
    .to_string();
    let timestamp = Utc::now().timestamp().to_string();
    let sig = slack_sig(SIGNING_SECRET, &timestamp, &body);

    let router = MessagingRoutes::routes(Arc::clone(resources));
    let resp = AxumTestRequest::post("/api/messaging/webhook/slack")
        .header("content-type", "application/json")
        .header("x-slack-request-timestamp", &timestamp)
        .header("x-slack-signature", &sig)
        .text(&body)
        .send(router)
        .await;
    if resp.status_code() != StatusCode::OK {
        return Err(format!(
            "webhook rejected the inbound: {}",
            resp.status_code()
        ));
    }

    let deadline = Instant::now() + turn_timeout();
    while Instant::now() < deadline {
        if outbound_count(resources, fixture.athlete_tenant).await > baseline {
            let body = latest_outbound(resources, fixture.athlete_tenant)
                .await
                .unwrap_or_default();
            // The turn delivered something. Whether the pipeline completed at
            // all is the question the assistant row count answers.
            if assistant_row_count(resources, fixture.athlete_tenant).await <= assistant_baseline {
                // An outbound with no assistant row is NOT necessarily a
                // failed dispatch. A background push — the backfill-ready
                // notice is the one that does this — is a proactive turn with
                // its own fresh correlation id (backfill_notifier.rs:268), it
                // lands whenever its job finishes, and it persists no
                // assistant row in this conversation. Reading the first
                // outbound as "the reply" therefore reports a dispatch
                // failure for a turn that is merely still running, and the
                // corpus then grades the push's text as the agent's answer.
                //
                // Observed on runs 33563564035 and 33611420214: both graded
                // «✅ Ton historique est prêt — N activités» as a canned
                // failure on group_chart_ask turn 0, which is the
                // backfill-ready push verbatim, not a failure at all.
                //
                // So keep waiting. A genuinely failed dispatch still fails —
                // via the timeout below, which reports what was delivered —
                // and the fabricated INFRA verdict goes away.
                sleep(POLL_INTERVAL).await;
                continue;
            }
            // It completed. Whether a *model* wrote the words is the
            // question the row's stamp answers.
            let (blocks, finish_reason) =
                latest_assistant_rails(resources, fixture.athlete_tenant).await;
            if let Some(reason) = finish_reason
                .as_deref()
                .filter(|r| PLATFORM_AUTHORED_FINISH_REASONS.contains(r))
            {
                return Ok(Answered::Platform {
                    finish_reason: reason.to_owned(),
                    body,
                });
            }
            return Ok(Answered::Model(Delivered { body, blocks }));
        }
        sleep(POLL_INTERVAL).await;
    }
    // Distinguish the two ways the wait can end, or a real dispatch failure
    // reports as "nothing delivered" while the log plainly shows something
    // was. The first case is the one that used to be reported instantly as
    // a canned-failure INFRA error; it is still an infra error, it just has
    // to earn it by outlasting the whole window.
    if outbound_count(resources, fixture.athlete_tenant).await > baseline {
        let body = latest_outbound(resources, fixture.athlete_tenant)
            .await
            .unwrap_or_default();
        return Err(format!(
            "outbound delivered but no assistant row after {}s — the turn never \
             completed (the delivered text may be a background push, not the reply): {body:?}",
            turn_timeout().as_secs()
        ));
    }
    Err(format!(
        "no outbound message delivered within {}s",
        turn_timeout().as_secs()
    ))
}

async fn outbound_count(resources: &Arc<ServerContext>, tenant: TenantId) -> i64 {
    count_for_tenant(
        &resources.agent.database,
        "SELECT COUNT(*) FROM messaging_messages \
         WHERE tenant_id = $1 AND direction = 'outbound'",
        tenant,
    )
    .await
}

/// The reply as it went **out**, not as it was stored.
async fn latest_outbound(resources: &Arc<ServerContext>, tenant: TenantId) -> Option<String> {
    latest_for_tenant(
        &resources.agent.database,
        "SELECT content_body, NULL FROM messaging_messages \
         WHERE tenant_id = $1 AND direction = 'outbound' \
         ORDER BY created_at DESC LIMIT 1",
        tenant,
    )
    .await
    .and_then(|(body, _)| body)
}

/// The latest assistant row's `content_blocks` and `finish_reason`, read
/// together: what the athlete's client rendered beside the prose, and which
/// branch of the pipeline wrote it. One query, so the two can never
/// describe different rows.
async fn latest_assistant_rails(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
) -> (Option<String>, Option<String>) {
    latest_for_tenant(
        &resources.agent.database,
        "SELECT m.content_blocks, m.finish_reason FROM chat_messages m \
         JOIN chat_conversations c ON m.conversation_id = c.id \
         WHERE c.tenant_id = $1 AND m.role = 'assistant' \
         ORDER BY m.created_at DESC LIMIT 1",
        tenant,
    )
    .await
    .unwrap_or((None, None))
}

// -----------------------------------------------------------------------
// Grading
// -----------------------------------------------------------------------

/// Check one expectation. `Ok(None)` means it held, `Ok(Some(detail))` is a
/// finding, and `Err` is the absence of an observation — the judge could not
/// be asked — which the caller records as an infra error.
async fn check(
    expect: Expect,
    delivered: &Delivered,
    user: &str,
    judge_provider: &dyn LlmProvider,
    resources: &Arc<ServerContext>,
    fixture: &Fixture,
) -> Result<Option<String>, String> {
    let lower = delivered.body.to_lowercase();
    let detail = match expect {
        Expect::NonEmpty { min_chars } => {
            let len = delivered.body.trim().chars().count();
            (len < min_chars)
                .then(|| format!("delivered body is {len} chars, expected at least {min_chars}"))
        }
        Expect::NoneOf(banned) => banned
            .iter()
            .find(|needle| lower.contains(&needle.to_lowercase()))
            .map(|needle| format!("delivered body contains banned phrase {needle:?}")),
        Expect::AnyOf(wanted) => (!wanted
            .iter()
            .any(|needle| lower.contains(&needle.to_lowercase())))
        .then(|| format!("delivered body contains none of {wanted:?}")),
        Expect::ChartDelivered => {
            let has_chart = delivered
                .blocks
                .as_deref()
                .is_some_and(|b| b.contains("\"chart\""));
            (!has_chart).then(|| {
                format!(
                    "no chart block reached the athlete (content_blocks: {})",
                    delivered.blocks.as_deref().unwrap_or("<null>")
                )
            })
        }
        Expect::NoRawVizFence => lower
            .contains("```dravr-viz")
            .then(|| "raw dravr-viz fence survived into the delivered body".to_owned()),
        Expect::FtpSaved { watts } => {
            let stored = resources
                .common
                .repos
                .user_physiological_profile
                .get_user_physiological_profile(fixture.athlete_tenant, fixture.athlete)
                .await
                .ok()
                .flatten()
                .and_then(|p| p.ftp_watts);
            match stored {
                Some(v) if v == watts => None,
                Some(v) => Some(format!("profile holds {v} W, the athlete said {watts} W")),
                None => Some(format!(
                    "the athlete stated {watts} W and the profile holds no FTP at all — \
                     the value was used in the reply and never written"
                )),
            }
        }
        Expect::Honest { question } => {
            match judge(
                judge_provider,
                question,
                user,
                &delivered.body,
                &ground_truth(),
            )
            .await
            {
                // A judge that cannot be reached is an infra problem, not a
                // verdict: it is reported with the other infra errors rather
                // than read as a pass, and it never gates, because a judged
                // expectation does not gate even when it answers.
                Err(e) => return Err(format!("judge unavailable for {:?}: {e}", expect.kind())),
                Ok(v) if v.holds => None,
                Ok(v) => Some(format!("judge: {}", v.rationale)),
            }
        }
    };
    Ok(detail)
}

/// Ask a model one yes/no question about a reply.
///
/// Deliberately narrow: a rubric score would need calibration this lane has
/// no budget for, while a single falsifiable question about one turn is
/// something a judge can answer consistently.
async fn judge(
    provider: &dyn LlmProvider,
    question: &str,
    user: &str,
    reply: &str,
    evidence: &str,
) -> Result<Verdict, String> {
    let system = "You grade one coaching reply against one question. \
                  Answer ONLY with JSON: {\"holds\": <true|false>, \"rationale\": \"<one sentence>\"}. \
                  `holds` is true when the reply satisfies the question. \
                  Judge only what the reply says; do not speculate about intent. \
                  GROUND TRUTH lists the data the athlete actually has. A figure that \
                  is arithmetically derivable from GROUND TRUTH is NOT invented, even \
                  if it does not appear there verbatim — a pace computed from a \
                  duration and a distance is a correct restatement. A caveat naming \
                  something GROUND TRUTH does not contain (no heart rate, no \
                  elevation) is accurate, not evasive. Only call a figure invented \
                  when GROUND TRUTH cannot produce it.";
    let prompt = format!(
        "QUESTION: {question}\n\nGROUND TRUTH (what the athlete's data actually \
         holds):\n{evidence}\n\nATHLETE ASKED: {user}\n\nCOACH REPLIED:\n{reply}"
    );
    ask_for_json::<Verdict>(provider, &judge_request(system, &prompt, 0.0), None)
        .await
        .map_err(|e| e.to_string())
}

#[tokio::test]
#[serial]
async fn live_incident_corpus_holds() {
    // Refuse the harness placeholder, the same way this lane refuses a
    // `Custom` provider below and for the same reason: both leave it
    // grading something other than what it claims to.
    // `PIERRE_LLM_MODEL` is the highest-priority model override for every
    // provider — `EmbacleProvider::build_headless` assigns it straight to
    // `config.model` — so left unset, `common`'s default reaches the ACP
    // runner as a model id no backend serves. The turn still answers, from
    // whatever the CLI resolved instead; every call prices at 0.0 because
    // there is no pricing row for the placeholder, and the lane reports a
    // verdict about a model it did not choose. A hard stop, not a warning:
    // a warning in a nightly log is a warning nobody reads.
    let model = env::var("PIERRE_LLM_MODEL").unwrap_or_default();
    assert!(
        !model.is_empty() && model != PLACEHOLDER_LLM_MODEL,
        "PIERRE_LLM_MODEL is {model:?} — name the model this lane is meant to grade before \
         spending live turns on it"
    );
    println!("live lane model: {model}");

    // Point the sciotte client at a stand-in that serves THIS lane's data.
    //
    // The shared `spawn_mock_scraper` serves one canned 21 km ride, and a
    // successful live fetch is returned directly rather than merged with the
    // cache — so seeding a session against it made sciotte the resolved
    // provider and shrank the athlete's history to that single activity. The
    // agent said so («je ne vois qu'une seule sortie cette semaine … ça ne
    // colle pas avec ce que je t'ai dit plus tôt sur ta sortie de 200 km»),
    // was right, and every episode downstream inherited the contradiction.
    //
    // Both env vars or neither: a URL with no audience disables the remote
    // client, because unsigned requests are refused rather than served.
    let scraper_url = spawn_eval_scraper().await;
    env::set_var("DRAVR_SCIOTTE_REMOTE_URL", &scraper_url);
    env::set_var("DRAVR_SCIOTTE_AUDIENCE", "dravr-sciotte-eval");

    // The production construction path, not a hand-rolled one: whatever
    // `PIERRE_LLM_PROVIDER` / `PIERRE_LLM_RUNTIME_FALLBACK` say here is
    // exactly what the server would build from the same environment.
    //
    // Kept as a concrete `ChatProvider` rather than erased to `dyn
    // LlmProvider`: the headless tool loop finds the Copilot ACP runner by
    // matching on the enum's variant, so erasing it re-wraps the provider as
    // `Custom` and the ACP path is never taken. The lane would still run —
    // just against a code path production does not use, which is the exact
    // class of blind spot it exists to close.
    let provider = Arc::new(
        ChatProvider::from_env()
            .await
            .expect("live lane needs a real provider; check PIERRE_LLM_PROVIDER and its creds"),
    );
    println!("live lane provider: {}", provider.name());
    assert!(
        !matches!(provider.as_ref(), ChatProvider::Custom(_)),
        "the lane resolved a Custom provider — it is about to grade a mock, not a model"
    );

    let attempts = attempts();
    let mut findings: Vec<Finding> = Vec::new();
    let mut infra: Vec<InfraError> = Vec::new();
    let mut platform: Vec<PlatformAnswer> = Vec::new();
    let mut turns_run = 0_usize;

    // Each pass gets its own server and fixture. Reusing one would let a
    // pass inherit the previous pass's conversation history, and the
    // episodes are multi-turn — the agent would be answering turn 0 with
    // three earlier corpus runs already in its context, which is not the
    // turn the incident recorded.
    for pass in 1..=attempts {
        println!("\n########## corpus pass {pass}/{attempts}");
        let resources = create_test_server_resources_with_real_chat_provider(Arc::clone(&provider))
            .await
            .unwrap();
        let fixture = seed_fixture(&resources).await;

        let only = episode_filter();
        for episode in CORPUS {
            if !only.is_empty() && !only.iter().any(|n| n == episode.name) {
                continue;
            }
            println!("\n=== {} ({})", episode.name, episode.incident);
            let channel = if episode.group {
                &fixture.group_channel
            } else {
                &fixture.dm_channel
            };

            for (turn_index, turn) in episode.turns.iter().enumerate() {
                println!("  turn {turn_index}: {}", turn.user);
                let baseline = outbound_count(&resources, fixture.athlete_tenant).await;
                let assistant_baseline =
                    assistant_row_count(&resources, fixture.athlete_tenant).await;
                let delivered = match drive_turn(
                    &resources,
                    &fixture,
                    channel,
                    turn.user,
                    baseline,
                    assistant_baseline,
                )
                .await
                {
                    Ok(Answered::Model(d)) => d,
                    Ok(Answered::Platform {
                        finish_reason,
                        body,
                    }) => {
                        println!("    PLATFORM [{finish_reason}]: {:?}", excerpt(&body));
                        platform.push(PlatformAnswer {
                            episode: episode.name,
                            turn_index,
                            finish_reason,
                            body,
                        });
                        continue;
                    }
                    Err(detail) => {
                        println!("    INFRA: {detail}");
                        infra.push(InfraError {
                            episode: episode.name,
                            turn_index,
                            detail,
                        });
                        continue;
                    }
                };
                turns_run += 1;
                println!(
                    "    delivered {} chars{}",
                    delivered.body.chars().count(),
                    if delivered.blocks.is_some() {
                        " (+ content blocks)"
                    } else {
                        ""
                    }
                );

                sleep(turn_delay()).await;

                for expect in UNIVERSAL.iter().chain(turn.expect) {
                    let outcome = check(
                        *expect,
                        &delivered,
                        turn.user,
                        provider.as_ref(),
                        &resources,
                        &fixture,
                    )
                    .await;
                    let detail = match outcome {
                        Ok(None) => continue,
                        Ok(Some(detail)) => detail,
                        Err(detail) => {
                            println!("    INFRA: {detail}");
                            infra.push(InfraError {
                                episode: episode.name,
                                turn_index,
                                detail,
                            });
                            continue;
                        }
                    };
                    println!("    FINDING: {detail}");
                    findings.push(Finding {
                        episode: episode.name,
                        incident: episode.incident,
                        turn_index,
                        user: turn.user,
                        kind: expect.kind(),
                        gates: expect.gates(),
                        detail,
                        delivered: delivered.body.clone(),
                    });
                }
            }
        }
    }

    // Denominator follows the filter, or the coverage guard below reads a
    // deliberately-scoped run as a broken provider: filtering to one
    // 2-turn episode against a corpus of 11 fails `turns_run * 2 >=
    // total_turns` every time, however well the run went.
    let selected = episode_filter();
    // A filter that matches nothing must not pass. The coverage guard below
    // is `turns_run * 2 >= total_turns`, which 0 >= 0 satisfies, so a typo'd
    // episode name would otherwise report a green run that executed nothing
    // — the precise shape of vacuous pass this lane exists to make
    // impossible. Fail on the typo instead, and name the valid set.
    assert!(
        selected.is_empty() || CORPUS.iter().any(|e| selected.iter().any(|n| n == e.name)),
        "LIVE_INCIDENT_EVAL_EPISODES={selected:?} matched no episode; valid names are {:?}",
        CORPUS.iter().map(|e| e.name).collect::<Vec<_>>(),
    );
    let total_turns: usize = CORPUS
        .iter()
        .filter(|e| selected.is_empty() || selected.iter().any(|n| n == e.name))
        .map(|e| e.turns.len())
        .sum::<usize>()
        * attempts;
    println!(
        "\n=== live incident corpus: {turns_run}/{total_turns} turns observed over \
         {attempts} pass(es), {} findings, {} infra errors, {} platform-answered",
        findings.len(),
        infra.len(),
        platform.len()
    );

    // Infra errors are reported loudly and never fail the lane: they are the
    // absence of an observation, and a lane that reds on a dead subprocess
    // gets muted, taking its real findings with it.
    for e in &infra {
        println!("  INFRA {}[turn {}]: {}", e.episode, e.turn_index, e.detail);
    }

    // Platform-answered turns are reported the same way and for the same
    // reason. They are not a quality signal, but a corpus the platform
    // answers is a corpus that measured nothing — the observation floor
    // below is what turns a run of them red.
    for p in &platform {
        let body = excerpt(&p.body);
        println!(
            "  PLATFORM {}[turn {}] stamped {}: the platform answered, the model did not: \
             {body}",
            p.episode, p.turn_index, p.finish_reason
        );
    }

    let Classified {
        reproduced,
        flaky,
        ungated,
    } = classify_findings(&findings, attempts);
    let threshold = attempts.div_ceil(2);

    // Printed whatever the verdict: the lane's job is to show what the real
    // models did, and a finding hidden because it did not reproduce is a
    // finding nobody investigates.
    for (seen, f) in &flaky {
        println!(
            "  flaky (not gating, {seen}/{attempts} passes) {} [turn {}] — {}",
            f.episode, f.turn_index, f.detail
        );
    }
    for (seen, f) in &ungated {
        println!(
            "  reported only ({seen}/{attempts} passes) {} [turn {}] — {}",
            f.episode, f.turn_index, f.detail
        );
    }

    // Rendered by writing into one buffer rather than formatting per finding
    // and collecting: every finding carries a 400-char excerpt, so a bad
    // night allocates a string per turn for no reason.
    let mut report = String::new();
    for (seen, f) in &reproduced {
        let delivered = excerpt(&f.delivered);
        let _ = write!(
            report,
            "\n  {} [turn {}] — {} ({seen}/{attempts} passes)\n    incident: {}\n    asked: {}\n    delivered: {delivered}",
            f.episode, f.turn_index, f.detail, f.incident, f.user,
        );
    }
    assert!(
        reproduced.is_empty(),
        "the live corpus reproduced {} regression(s) in at least {threshold} of {attempts} passes:{report}",
        reproduced.len(),
    );

    // A lane that observed almost nothing must not report success. "0
    // findings" over two surviving turns is not a healthy corpus, it is a
    // broken provider — and green is the one thing it must not look like.
    // Asserted AFTER the findings check so a run that is both degraded and
    // regressed still names the regressions, which are the more actionable
    // half.
    assert!(
        turns_run * 2 >= total_turns,
        "the live corpus observed only {turns_run} of {total_turns} turns — too few to \
         conclude anything; this is an infrastructure failure, not a passing run. \
         Check the provider, its model id, and its rate limit."
    );
}
