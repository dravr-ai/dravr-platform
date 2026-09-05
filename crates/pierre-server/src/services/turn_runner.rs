// ABOUTME: How a messaging turn is started — in this process, or as a Cloud Tasks request the backend receives
// ABOUTME: Runner selection from the environment at boot, and the Cloud Tasks enqueuer with the OIDC verifier its route uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one place a messaging turn is started.
//!
//! A turn that runs detached from any request is invisible to Cloud Run's
//! liveness accounting: the webhook answered 200, so the instance reads as
//! idle and a scaledown may take the turn (registre#126). The
//! [`TurnRunner`] is the one place a turn is started, and on GCP it does not
//! run the turn at all — it enqueues a Cloud Tasks task whose HTTP target is
//! this service's own `/internal/turns/{id}/run`. Cloud Tasks delivers that
//! as a real request, the turn runs inside it, and an instance processing a
//! request is one Cloud Run waits for. Locally and in tests the runner is
//! [`TurnRunner::InProcess`], which spawns the turn through the in-flight
//! tracker exactly as the webhook used to.
//!
//! The runner is chosen once at boot from the environment and never per
//! turn: a deployment either has a queue or it does not. A turn the queue
//! refuses is left on file for the sweep to enqueue again; it is never run in
//! this process instead.

use std::env;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as Base64Standard;
use base64::Engine as _;
use pierre_auth::google_id_token::{GoogleIdTokenVerifier, GOOGLE_OIDC_CERTS_URL};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use pierre_core::http_client::api_client;
use pierre_core::models::TenantId;
use serde_json::json;
use tracing::{info, warn};

/// Selects the runner: `in_process` (the default when absent) or `cloud_tasks`.
pub const ENV_TURN_RUNNER: &str = "PIERRE_TURN_RUNNER";
/// The queue, as `projects/{p}/locations/{l}/queues/{q}`.
pub const ENV_TURN_QUEUE: &str = "PIERRE_TURN_QUEUE";
/// This service's own `run.app` origin, with no path: the task target and
/// the token audience.
pub const ENV_TURN_TARGET_URL: &str = "PIERRE_TURN_TARGET_URL";
/// The service account each task's OIDC token is minted on behalf of.
pub const ENV_TURN_OIDC_SERVICE_ACCOUNT: &str = "PIERRE_TURN_OIDC_SERVICE_ACCOUNT";
/// How long the run route waits, inside the request, for a blocked claim.
pub const ENV_TURN_CLAIM_WAIT_SECS: &str = "PIERRE_TURN_CLAIM_WAIT_SECS";

/// The Cloud Tasks REST API.
pub const CLOUD_TASKS_API_BASE: &str = "https://cloudtasks.googleapis.com/v2";

/// Default for [`ENV_TURN_CLAIM_WAIT_SECS`]: long enough for a predecessor
/// turn in the same conversation to finish an ordinary answer, short enough
/// that a task never sits in a request for the whole dispatch deadline.
const DEFAULT_CLAIM_WAIT: Duration = Duration::from_mins(4);

/// Slack added to the turn watchdog for the task's dispatch deadline, so the
/// deadline never cuts a turn the watchdog would still allow.
const DISPATCH_DEADLINE_MARGIN: Duration = Duration::from_mins(1);

/// Cloud Tasks caps an HTTP task's dispatch deadline at thirty minutes.
const CLOUD_TASKS_MAX_DISPATCH_DEADLINE: Duration = Duration::from_mins(30);

/// Everything the Cloud Tasks runner needs to know.
#[derive(Debug, Clone)]
pub struct CloudTasksConfig {
    /// The queue, as `projects/{p}/locations/{l}/queues/{q}`.
    pub queue: String,
    /// This service's `run.app` origin, no path.
    pub target_url: String,
    /// The service account the tasks' OIDC tokens are minted on behalf of.
    pub service_account: String,
    /// How long the run route waits for a blocked claim.
    pub claim_wait: Duration,
    /// Cloud Tasks API base; overridden by tests to a local listener.
    pub api_base: String,
    /// Google certificate endpoint the verifier reads; overridden by tests.
    pub certs_url: String,
}

/// Where a messaging turn runs.
pub enum TurnRunner {
    /// Spawned through the in-flight tracker in this process.
    InProcess,
    /// Enqueued on Cloud Tasks and run inside the request it delivers. Boxed:
    /// the runner carries a verifier and its certificate cache, the other arm
    /// nothing.
    CloudTasks(Box<CloudTasksRunner>),
}

impl TurnRunner {
    /// The runner the environment selects, minting Cloud Tasks credentials
    /// from the metadata server.
    ///
    /// # Errors
    ///
    /// Returns an error when `cloud_tasks` is selected without its queue,
    /// target URL or service account, when the claim wait is not a number,
    /// or when the turn watchdog plus its margin exceeds what Cloud Tasks
    /// accepts as a dispatch deadline.
    pub fn from_env(turn_watchdog: Duration) -> AppResult<Self> {
        Self::parse(
            |key| env::var(key).ok(),
            turn_watchdog,
            Arc::new(MetadataTokenProvider::default()),
        )
    }

    /// The runner a set of settings selects. `lookup` answers the
    /// [`ENV_TURN_RUNNER`] family of names; absent means in-process.
    ///
    /// # Errors
    ///
    /// See [`Self::from_env`].
    pub fn parse(
        lookup: impl Fn(&str) -> Option<String>,
        turn_watchdog: Duration,
        token_provider: Arc<dyn TokenProvider>,
    ) -> AppResult<Self> {
        let selected = lookup(ENV_TURN_RUNNER).unwrap_or_else(|| "in_process".to_owned());
        match selected.as_str() {
            "in_process" => Ok(Self::InProcess),
            "cloud_tasks" => {
                let required = |name: &str| {
                    lookup(name).filter(|v| !v.trim().is_empty()).ok_or_else(|| {
                        AppError::internal(format!(
                            "{ENV_TURN_RUNNER}=cloud_tasks needs {name}; unset {ENV_TURN_RUNNER} to run turns in-process"
                        ))
                    })
                };
                let claim_wait = match lookup(ENV_TURN_CLAIM_WAIT_SECS) {
                    None => DEFAULT_CLAIM_WAIT,
                    Some(raw) => {
                        raw.trim()
                            .parse::<u64>()
                            .map(Duration::from_secs)
                            .map_err(|_| {
                                AppError::internal(format!(
                            "{ENV_TURN_CLAIM_WAIT_SECS} must be a number of seconds, got {raw:?}"
                        ))
                            })?
                    }
                };
                let config = CloudTasksConfig {
                    queue: required(ENV_TURN_QUEUE)?,
                    target_url: required(ENV_TURN_TARGET_URL)?
                        .trim_end_matches('/')
                        .to_owned(),
                    service_account: required(ENV_TURN_OIDC_SERVICE_ACCOUNT)?,
                    claim_wait,
                    api_base: CLOUD_TASKS_API_BASE.to_owned(),
                    certs_url: GOOGLE_OIDC_CERTS_URL.to_owned(),
                };
                Ok(Self::CloudTasks(Box::new(CloudTasksRunner::new(
                    config,
                    token_provider,
                    turn_watchdog,
                )?)))
            }
            other => Err(AppError::internal(format!(
                "{ENV_TURN_RUNNER} must be in_process or cloud_tasks, got {other:?}"
            ))),
        }
    }

    /// The Cloud Tasks runner, when that is the runner in use.
    #[must_use]
    pub const fn cloud_tasks(&self) -> Option<&CloudTasksRunner> {
        match self {
            Self::InProcess => None,
            Self::CloudTasks(runner) => Some(runner),
        }
    }

    /// Stable label for logs.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::CloudTasks(_) => "cloud_tasks",
        }
    }
}

impl fmt::Debug for TurnRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InProcess => f.write_str("TurnRunner::InProcess"),
            Self::CloudTasks(runner) => f
                .debug_struct("TurnRunner::CloudTasks")
                .field("queue", &runner.config.queue)
                .field("target_url", &runner.config.target_url)
                .field("service_account", &runner.config.service_account)
                .field("dispatch_deadline", &runner.dispatch_deadline)
                .finish(),
        }
    }
}

/// Enqueues turns on Cloud Tasks and verifies the tokens their deliveries carry.
pub struct CloudTasksRunner {
    config: CloudTasksConfig,
    /// Bearer token source for the Cloud Tasks API — the metadata server in
    /// production, a stub in tests.
    token_provider: Arc<dyn TokenProvider>,
    verifier: GoogleIdTokenVerifier,
    dispatch_deadline: Duration,
}

impl CloudTasksRunner {
    /// A runner over `config`, with the dispatch deadline derived from the
    /// turn watchdog.
    ///
    /// # Errors
    ///
    /// Returns an error when the watchdog plus its margin exceeds the
    /// deadline Cloud Tasks accepts.
    pub fn new(
        config: CloudTasksConfig,
        token_provider: Arc<dyn TokenProvider>,
        turn_watchdog: Duration,
    ) -> AppResult<Self> {
        let dispatch_deadline = turn_watchdog + DISPATCH_DEADLINE_MARGIN;
        if dispatch_deadline > CLOUD_TASKS_MAX_DISPATCH_DEADLINE {
            return Err(AppError::internal(format!(
                "the turn watchdog ({}s) plus {}s of margin exceeds the {}s dispatch deadline Cloud Tasks accepts; lower MESSAGING_TURN_WATCHDOG_SECS",
                turn_watchdog.as_secs(),
                DISPATCH_DEADLINE_MARGIN.as_secs(),
                CLOUD_TASKS_MAX_DISPATCH_DEADLINE.as_secs()
            )));
        }
        let verifier = GoogleIdTokenVerifier::with_certs_url(
            &config.target_url,
            &config.service_account,
            &config.certs_url,
        );
        Ok(Self {
            config,
            token_provider,
            verifier,
            dispatch_deadline,
        })
    }

    /// The verifier the run route checks each delivery's token with.
    #[must_use]
    pub const fn verifier(&self) -> &GoogleIdTokenVerifier {
        &self.verifier
    }

    /// How long the run route waits, inside the request, for a blocked claim.
    #[must_use]
    pub const fn claim_wait(&self) -> Duration {
        self.config.claim_wait
    }

    /// The dispatch deadline every task carries: the turn watchdog plus a
    /// minute, so Cloud Tasks never gives up on a turn the watchdog would
    /// still let finish.
    #[must_use]
    pub const fn dispatch_deadline(&self) -> Duration {
        self.dispatch_deadline
    }

    /// The URL a task for `turn_id` is delivered to.
    #[must_use]
    pub fn run_url(&self, turn_id: &str) -> String {
        format!("{}/internal/turns/{turn_id}/run", self.config.target_url)
    }

    /// The task name for the `seq`th enqueue of `turn_id`.
    ///
    /// A hashed prefix, as Google recommends for create latency; the row id;
    /// and the enqueue sequence, because a task name the queue has executed
    /// or deleted stays unusable for up to a day, and a re-enqueue under the
    /// old name would read as already done.
    #[must_use]
    pub fn task_name(&self, turn_id: &str, seq: i64) -> String {
        let mut hasher = DefaultHasher::new();
        turn_id.hash(&mut hasher);
        format!(
            "{}/tasks/{:016x}-{turn_id}-e{seq}",
            self.config.queue,
            hasher.finish()
        )
    }

    /// Enqueue the `seq`th delivery of `turn_id` for `tenant_id`.
    ///
    /// A task that already exists under this name is success: the same
    /// enqueue was retried. Any other refusal is an error the caller logs
    /// and leaves for the sweep, which enqueues again under the next
    /// sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when the bearer token cannot be minted, the API is
    /// unreachable, or it answers anything but success or `ALREADY_EXISTS`.
    pub async fn enqueue(&self, tenant_id: TenantId, turn_id: &str, seq: i64) -> AppResult<()> {
        let token = self.token_provider.access_token().await?;
        let body = json!({ "tenant_id": tenant_id.to_string() });
        let task = json!({
            "task": {
                "name": self.task_name(turn_id, seq),
                "dispatchDeadline": format!("{}s", self.dispatch_deadline.as_secs()),
                "httpRequest": {
                    "url": self.run_url(turn_id),
                    "httpMethod": "POST",
                    "headers": { "Content-Type": "application/json" },
                    "body": Base64Standard.encode(body.to_string()),
                    "oidcToken": {
                        "serviceAccountEmail": self.config.service_account,
                        "audience": self.config.target_url,
                    }
                }
            }
        });
        let url = format!("{}/{}/tasks", self.config.api_base, self.config.queue);
        let response = api_client()
            .post(&url)
            .bearer_auth(token)
            .json(&task)
            .send()
            .await
            .map_err(|e| {
                AppError::new(
                    ErrorCode::ExternalServiceUnavailable,
                    format!("Cloud Tasks unreachable: {e}"),
                )
            })?;
        let status = response.status();
        if status.is_success() {
            info!(turn_id, seq, "turn enqueued on Cloud Tasks");
            return Ok(());
        }
        if status.as_u16() == 409 {
            info!(
                turn_id,
                seq, "turn already enqueued on Cloud Tasks under this sequence"
            );
            return Ok(());
        }
        let detail = response.text().await.unwrap_or_default();
        warn!(turn_id, seq, status = status.as_u16(), detail = %detail, "Cloud Tasks refused the turn");
        Err(AppError::new(
            ErrorCode::ExternalServiceError,
            format!("Cloud Tasks answered HTTP {status} for turn {turn_id}"),
        ))
    }
}
