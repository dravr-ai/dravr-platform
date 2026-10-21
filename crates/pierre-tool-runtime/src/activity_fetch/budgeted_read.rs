// ABOUTME: A live activity-list read that answers its turn inside the refresh budget
// ABOUTME: Past the budget the cached window answers and the read runs on, so a slow scrape never fails a call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::{ai_scope, CoreFitnessProvider};
use tokio::task::{JoinError, JoinHandle};
use tokio::time::timeout;
use tracing::{info, warn, Instrument, Span};
use uuid::Uuid;

use super::sync_verdict::{HeadRead, LiveRead};
use super::{read_live_window, serve_stale_activities};
use crate::revalidation::revalidation_timeout;
use crate::runtime::ToolRuntime;

/// The owned half of a [`LiveRead`], so the read can outlive the turn that
/// started it.
pub struct OwnedLiveRead {
    /// The provider key the athlete's connection and cache rows use.
    pub provider_slug: String,
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant the connection lives in.
    pub tenant_id: TenantId,
    /// The window asked for.
    pub params: ActivityQueryParams,
    /// When the attempt began, before the credential was read.
    pub attempt_started_at: DateTime<Utc>,
}

/// How a budgeted live read ended for the turn that asked for it.
pub enum BudgetedRead {
    /// The read answered inside the budget, or ran to its end because nothing
    /// was cached to answer with instead. The provider comes back when the
    /// read did.
    Answered {
        /// What [`read_live_window`] returned.
        read: AppResult<HeadRead>,
        /// The authenticated provider, for the rest of the turn.
        provider: Option<Box<dyn CoreFitnessProvider>>,
    },
    /// The read outlived the budget and the provider's own cached window
    /// answered. The read runs on detached, capped at
    /// [`revalidation_timeout`], and its write-through lands for the next turn.
    Cached(Vec<Activity>),
}

/// What the detached read hands back: `None` when it hit its cap.
type ReadOutput = Option<(AppResult<HeadRead>, Box<dyn CoreFitnessProvider>)>;

/// Read a provider's window live, waiting at most `budget` for it.
///
/// A healthy scrape takes about two minutes, and the chat tool loop gives a
/// tool call ninety seconds: an inline read of a scraped provider could fail
/// the whole `get_activities` call on a provider that was working. Past the
/// budget the turn is answered from the provider's cached window, which the
/// caller labels stale, while the read continues; it is judged, recorded and
/// written through exactly as an inline read would be. With nothing cached
/// there is nothing better to say, so the turn keeps waiting for the read.
///
/// The provider-terms and AI-consent gates are task-locals of the turn, and a
/// spawned task does not inherit them: the detached read runs unfiltered, as a
/// cache write does, and what it answers is filtered here, back in the turn.
pub async fn read_live_window_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    provider: Box<dyn CoreFitnessProvider>,
    read: OwnedLiveRead,
    budget: Duration,
) -> BudgetedRead {
    let (provider_slug, user_id, tenant_id, params) = (
        read.provider_slug.clone(),
        read.user_id,
        read.tenant_id,
        read.params.clone(),
    );
    let mut task = spawn_read(Arc::clone(runtime), provider, read);
    if let Ok(joined) = timeout(budget, &mut task).await {
        return answered(runtime, joined);
    }
    if let Some(cached) =
        serve_stale_activities(runtime, &provider_slug, user_id, tenant_id, &params).await
    {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            budget_secs = budget.as_secs(),
            "live activity read outlived the turn's budget; answering from the cached window while it finishes"
        );
        observe_detached(task, user_id, provider_slug);
        return BudgetedRead::Cached(cached);
    }
    answered(runtime, task.await)
}

fn spawn_read(
    runtime: Arc<dyn ToolRuntime>,
    provider: Box<dyn CoreFitnessProvider>,
    read: OwnedLiveRead,
) -> JoinHandle<ReadOutput> {
    // The read keeps the turn's span, so its logs stay on the turn that
    // started it after the turn has answered.
    let span = Span::current();
    tokio::spawn(
        async move {
            let cap = revalidation_timeout(&runtime.config().timeouts);
            let tenant = read.tenant_id.to_string();
            let live = LiveRead {
                provider_slug: &read.provider_slug,
                user_id: read.user_id,
                tenant_id: Some(&tenant),
                params: &read.params,
                attempt_started_at: read.attempt_started_at,
            };
            let result = timeout(
                cap,
                ai_scope::unfiltered(read_live_window(
                    &runtime,
                    provider.as_ref(),
                    live,
                    Some(read.tenant_id),
                )),
            )
            .await;
            if result.is_err() {
                warn!(
                    user_id = %read.user_id,
                    provider = %read.provider_slug,
                    timeout_secs = cap.as_secs(),
                    "live activity read timed out"
                );
            }
            result.ok().map(|outcome| (outcome, provider))
        }
        .instrument(span),
    )
}

fn answered(runtime: &Arc<dyn ToolRuntime>, joined: Result<ReadOutput, JoinError>) -> BudgetedRead {
    match joined {
        Ok(Some((read, provider))) => BudgetedRead::Answered {
            read: read.map(|mut head| {
                head.activities = ai_scope::filter_activities(
                    runtime.provider_registry().as_ref(),
                    head.activities,
                );
                head
            }),
            provider: Some(provider),
        },
        Ok(None) => BudgetedRead::Answered {
            read: Err(AppError::external_service(
                "activity provider",
                format!(
                    "the live read did not answer within {}s",
                    revalidation_timeout(&runtime.config().timeouts).as_secs()
                ),
            )),
            provider: None,
        },
        Err(join_error) => BudgetedRead::Answered {
            read: Err(AppError::internal(format!(
                "live activity read failed: {join_error}"
            ))),
            provider: None,
        },
    }
}

/// Watch a read the turn stopped waiting for, so a panic in it is logged
/// rather than lost with its dropped handle.
fn observe_detached<T: Send + 'static>(task: JoinHandle<T>, user_id: Uuid, provider: String) {
    tokio::spawn(async move {
        if let Err(join_error) = task.await {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %join_error,
                "detached live activity read failed"
            );
        }
    });
}

/// Re-read a provider's recent head detached, waiting at most `budget` for it.
///
/// The head top-up of a covered historical window ([`super::refresh_stale_head`])
/// awaited [`super::refresh_head`] inline, so a slow scrape held the whole
/// call. Past the budget the read runs on under [`revalidation_timeout`] and
/// its write-through lands for the next turn; `None` tells the caller the head
/// it serves is behind. Run unfiltered and filtered here, for the reason
/// [`read_live_window_within_budget`] gives.
pub async fn refresh_head_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    budget: Duration,
) -> Option<Vec<Activity>> {
    let span = Span::current();
    let detached = Arc::clone(runtime);
    let slug = provider_slug.to_owned();
    let mut task = tokio::spawn(
        async move {
            let refresh = super::refresh_head(&detached, &slug, user_id, tenant_id);
            timeout(
                revalidation_timeout(&detached.config().timeouts),
                ai_scope::unfiltered(refresh),
            )
            .await
            .ok()
            .flatten()
        }
        .instrument(span),
    );
    let Ok(joined) = timeout(budget, &mut task).await else {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            budget_secs = budget.as_secs(),
            "stale activity head did not refresh within the turn's budget; serving the cached head while it finishes"
        );
        observe_detached(task, user_id, provider_slug.to_owned());
        return None;
    };
    let rows = match joined {
        Ok(rows) => rows?,
        Err(join_error) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %join_error,
                "stale activity head refresh task failed"
            );
            return None;
        }
    };
    Some(ai_scope::filter_activities(
        runtime.provider_registry().as_ref(),
        rows,
    ))
}
