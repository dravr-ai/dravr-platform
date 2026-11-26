// ABOUTME: Queues the check that follows a refused provider sign-in: a Cloud Tasks task on the turns queue, or a local timer
// ABOUTME: The composition root picks one for the auth routes, the way the routine engine picks its waker
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The production half of [`pierre_routes_auth::sign_in_follow_up`]
//! (carnet#1837).
//!
//! The live environment scales to zero, so a follow-up due ten minutes after
//! a refusal cannot wait in the process: it is a Cloud Tasks HTTP task on the
//! queue turns and routine wakes use, scheduled for its time and delivered
//! to [`FOLLOW_UP_PATH`] with the same OIDC identity, so it needs no queue,
//! binding or setting of its own. Each refusal names its own task
//! ([`FollowUp::task_id`]), and only the latest refusal's task acts.
//!
//! Wherever turns run in-process (locally), the process holds the follow-up
//! as a timer ([`InProcessFollowUps`]).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::TokenProvider;
use pierre_database::RepositoryRegistry;
use pierre_routes_auth::sign_in_follow_up::{
    FollowUp, InProcessFollowUps, SignInFollowUps, FOLLOW_UP_PATH,
};
use tracing::{info, warn};

use crate::services::cloud_tasks::{create_http_task, HttpTask, TaskCreated};
use crate::services::turn_runner::{CloudTasksRunner, TurnRunner};

/// The follow-ups the auth routes queue on this server.
///
/// Cloud Tasks tasks where turns run on Cloud Tasks and the delivery route
/// is mounted (the messaging router mounts it), a timer in the process
/// elsewhere.
#[must_use]
pub fn sign_in_follow_ups(
    turn_runner: &TurnRunner,
    cache: &Arc<Cache>,
    repos: &Arc<RepositoryRegistry>,
) -> Arc<dyn SignInFollowUps> {
    match turn_runner.cloud_tasks() {
        Some(runner) if cfg!(feature = "client-messaging") => {
            Arc::new(CloudTasksFollowUps::over(runner))
        }
        _ => Arc::new(InProcessFollowUps::new(
            Arc::clone(cache),
            Arc::clone(repos),
        )),
    }
}

/// Production [`SignInFollowUps`]: one Cloud Tasks HTTP task per refusal.
///
/// On the queue turns use, delivered to [`FOLLOW_UP_PATH`] with the same
/// OIDC identity.
pub struct CloudTasksFollowUps {
    queue: String,
    target_url: String,
    service_account: String,
    api_base: String,
    /// The turn runner's own token source, shared with it.
    tokens: Arc<dyn TokenProvider>,
    dispatch_deadline: Duration,
}

impl fmt::Debug for CloudTasksFollowUps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudTasksFollowUps")
            .field("queue", &self.queue)
            .field("target_url", &self.target_url)
            .finish_non_exhaustive()
    }
}

impl CloudTasksFollowUps {
    /// Follow-ups on the turn runner's queue, target and identity.
    #[must_use]
    pub fn over(runner: &CloudTasksRunner) -> Self {
        let config = runner.config();
        Self {
            queue: config.queue.clone(),
            target_url: config.target_url.clone(),
            service_account: config.service_account.clone(),
            api_base: config.api_base.clone(),
            tokens: runner.token_provider(),
            dispatch_deadline: runner.dispatch_deadline(),
        }
    }
}

#[async_trait]
impl SignInFollowUps for CloudTasksFollowUps {
    async fn queue(&self, follow_up: FollowUp) -> AppResult<()> {
        let body = serde_json::to_value(&follow_up).map_err(|e| {
            AppError::internal(format!("a sign-in follow-up did not serialize: {e}"))
        })?;
        let task = HttpTask {
            queue: &self.queue,
            name: format!("{}/tasks/{}", self.queue, follow_up.task_id()),
            url: format!("{}{FOLLOW_UP_PATH}", self.target_url),
            body,
            service_account: &self.service_account,
            audience: &self.target_url,
            dispatch_deadline: self.dispatch_deadline,
            schedule_time: follow_up.due_at(),
        };
        match create_http_task(self.tokens.as_ref(), &self.api_base, &task).await {
            Ok(TaskCreated::Created) => {
                info!(task = %follow_up.task_id(), "sign-in follow-up queued");
                Ok(())
            }
            Ok(TaskCreated::AlreadyQueued) => Ok(()),
            Err(e) => {
                warn!(task = %follow_up.task_id(), error = %e, "Cloud Tasks refused a sign-in follow-up");
                Err(e)
            }
        }
    }
}
