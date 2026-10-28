// ABOUTME: How archive import steps are scheduled: a Cloud Tasks queue with one dispatch at a time, or this process locally
// ABOUTME: Each step names the next; a duplicate or stale delivery commits nothing, barrage's compare-and-set sees to it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Scheduling import steps
//!
//! An import is a download step then one step per chunk of rows, each naming
//! the next ([`super::ops::run_step`]). Deployed, every step is a Cloud Tasks
//! HTTP task to this service's `POST /internal/imports/{id}/step`
//! ([`ImportRunner::CloudTasks`]): it runs inside a request Cloud Run can see,
//! with the full server context the projection needs, and the queue's single
//! concurrent dispatch keeps one step running at a time across every athlete,
//! which bounds an import's load on the API instances (barrage decodes one
//! file at a time inside a step). A failed step answers an error and Cloud
//! Tasks retries it with backoff.
//!
//! Locally and in tests the steps run in this process, one after the other,
//! in a task of their own ([`ImportRunner::InProcess`]).

use std::env;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use dravr_tronc::iam::GOOGLE_OIDC_JWKS_URL;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::{MetadataTokenProvider, TokenProvider};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::cloud_tasks::{
    create_http_task, DeliveryVerifier, HttpTask, TaskCreated, CLOUD_TASKS_API_BASE,
};

use super::ops::run_step;

/// Selects the runner: `in_process` (the default when absent) or `cloud_tasks`.
pub const ENV_IMPORT_RUNNER: &str = "PIERRE_IMPORT_RUNNER";
/// The queue, as `projects/{p}/locations/{l}/queues/{q}`.
pub const ENV_IMPORT_QUEUE: &str = "PIERRE_IMPORT_QUEUE";
/// This service's own `run.app` origin, with no path: the task target and
/// the token audience.
pub const ENV_IMPORT_TARGET_URL: &str = "PIERRE_IMPORT_TARGET_URL";
/// The service account each task's OIDC token is minted on behalf of.
pub const ENV_IMPORT_OIDC_SERVICE_ACCOUNT: &str = "PIERRE_IMPORT_OIDC_SERVICE_ACCOUNT";

/// How long one step delivery may run: inside the backend's Cloud Run
/// request timeout (`request_timeout = "1260s"` in `infra/environments/dev`),
/// so Cloud Run never cuts a step Cloud Tasks still waits on. A chunk takes
/// seconds; the download is the long step, and an export of a few GB arrives
/// well inside it.
const STEP_DISPATCH_DEADLINE: Duration = Duration::from_mins(20);

/// How many times a step run in this process is tried before the import is
/// left where it stopped.
const IN_PROCESS_ATTEMPTS: u32 = 5;

/// The wait before an in-process step is tried again.
const IN_PROCESS_RETRY: Duration = Duration::from_secs(2);

/// One step of one import: the download when `expect_cursor` is `None`, else
/// the chunk of rows starting there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepTask {
    /// The import.
    pub import_id: Uuid,
    /// The tenant it imports into.
    pub tenant_id: Uuid,
    /// The athlete it imports for.
    pub user_id: Uuid,
    /// The cursor the chunk expects; `None` for the download.
    pub expect_cursor: Option<u64>,
}

/// Where import steps run.
pub enum ImportRunner {
    /// In this process, one after the other.
    InProcess,
    /// As Cloud Tasks deliveries to this service.
    CloudTasks(Box<ImportTasks>),
}

impl fmt::Debug for ImportRunner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl ImportRunner {
    /// The runner the environment selects.
    ///
    /// # Errors
    ///
    /// An internal error naming the setting a Cloud Tasks runner lacks, or an
    /// unknown runner.
    pub fn from_env() -> AppResult<Self> {
        Self::parse(
            |key| env::var(key).ok(),
            Arc::new(MetadataTokenProvider::default()),
        )
    }

    /// The runner `lookup` selects, minting Cloud Tasks tokens from `tokens`.
    ///
    /// # Errors
    ///
    /// As [`Self::from_env`].
    pub fn parse(
        lookup: impl Fn(&str) -> Option<String>,
        tokens: Arc<dyn TokenProvider>,
    ) -> AppResult<Self> {
        let selected = lookup(ENV_IMPORT_RUNNER).unwrap_or_else(|| "in_process".to_owned());
        match selected.as_str() {
            "in_process" => Ok(Self::InProcess),
            "cloud_tasks" => {
                let required = |name: &str| {
                    lookup(name)
                        .filter(|v| !v.trim().is_empty())
                        .ok_or_else(|| {
                            AppError::internal(format!(
                                "{ENV_IMPORT_RUNNER}=cloud_tasks needs {name}"
                            ))
                        })
                };
                let target_url = required(ENV_IMPORT_TARGET_URL)?
                    .trim_end_matches('/')
                    .to_owned();
                let service_account = required(ENV_IMPORT_OIDC_SERVICE_ACCOUNT)?;
                Ok(Self::CloudTasks(Box::new(ImportTasks {
                    queue: required(ENV_IMPORT_QUEUE)?,
                    verifier: DeliveryVerifier::new(
                        &target_url,
                        &service_account,
                        GOOGLE_OIDC_JWKS_URL,
                    ),
                    target_url,
                    service_account,
                    api_base: CLOUD_TASKS_API_BASE.to_owned(),
                    tokens,
                })))
            }
            other => Err(AppError::internal(format!(
                "{ENV_IMPORT_RUNNER} must be in_process or cloud_tasks, got {other:?}"
            ))),
        }
    }

    /// The runner's name, for the boot log.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::CloudTasks(_) => "cloud_tasks",
        }
    }

    /// The Cloud Tasks runner, when steps are delivered.
    #[must_use]
    pub const fn cloud_tasks(&self) -> Option<&ImportTasks> {
        match self {
            Self::InProcess => None,
            Self::CloudTasks(tasks) => Some(tasks),
        }
    }

    /// Schedule `task`.
    ///
    /// # Errors
    ///
    /// The Cloud Tasks error when the queue refuses the task; a step run in
    /// this process cannot fail to be scheduled.
    pub async fn schedule(&self, resources: &Arc<ServerContext>, task: StepTask) -> AppResult<()> {
        match self {
            Self::InProcess => {
                let resources = Arc::clone(resources);
                tokio::spawn(async move { drive_in_process(&resources, task).await });
                Ok(())
            }
            Self::CloudTasks(tasks) => tasks.enqueue(&task).await,
        }
    }
}

/// Run `task` and every step it leads to in this process, trying each a few
/// times before leaving the import where it stopped.
async fn drive_in_process(resources: &Arc<ServerContext>, first: StepTask) {
    let mut next = Some(first);
    while let Some(task) = next {
        let mut attempt = 1;
        next = loop {
            match run_step(resources, &task).await {
                Ok(after) => break after,
                Err(e) if attempt < IN_PROCESS_ATTEMPTS => {
                    warn!(import = %task.import_id, attempt, error = %e, "import step failed; trying it again");
                    attempt += 1;
                    sleep(IN_PROCESS_RETRY).await;
                }
                Err(e) => {
                    warn!(import = %task.import_id, error = %e, "import step failed on every attempt; the import stops here");
                    break None;
                }
            }
        };
    }
}

/// Enqueues import steps on Cloud Tasks and verifies their deliveries.
pub struct ImportTasks {
    queue: String,
    target_url: String,
    service_account: String,
    api_base: String,
    tokens: Arc<dyn TokenProvider>,
    verifier: DeliveryVerifier,
}

impl fmt::Debug for ImportTasks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportTasks")
            .field("queue", &self.queue)
            .field("target_url", &self.target_url)
            .field("service_account", &self.service_account)
            .finish_non_exhaustive()
    }
}

impl ImportTasks {
    /// Check the bearer token a step delivery carries.
    ///
    /// # Errors
    ///
    /// As [`DeliveryVerifier::verify`].
    pub async fn verify_delivery(&self, token: &str) -> AppResult<()> {
        self.verifier.verify(token).await
    }

    /// The URL a step of `import_id` is delivered to.
    #[must_use]
    pub fn step_url(&self, import_id: Uuid) -> String {
        format!("{}/internal/imports/{import_id}/step", self.target_url)
    }

    /// Enqueue `task`, named for its import and cursor: the same step
    /// enqueued twice (a stale delivery handing on the step its import is
    /// at, while that step is already queued) is one task, and a step
    /// delivered twice commits once, by barrage's compare-and-set.
    ///
    /// # Errors
    ///
    /// The Cloud Tasks error when the queue refuses the task.
    pub async fn enqueue(&self, task: &StepTask) -> AppResult<()> {
        let phase = task
            .expect_cursor
            .map_or_else(|| "download".to_owned(), |cursor| format!("c{cursor}"));
        let http = HttpTask {
            queue: &self.queue,
            name: format!(
                "{}/tasks/import-{}-{phase}",
                self.queue,
                task.import_id.simple()
            ),
            url: self.step_url(task.import_id),
            body: json!(task),
            service_account: &self.service_account,
            audience: &self.target_url,
            dispatch_deadline: STEP_DISPATCH_DEADLINE,
        };
        match create_http_task(self.tokens.as_ref(), &self.api_base, &http).await? {
            TaskCreated::Created | TaskCreated::AlreadyQueued => {
                info!(import = %task.import_id, phase = %phase, "import step enqueued");
                Ok(())
            }
        }
    }
}
