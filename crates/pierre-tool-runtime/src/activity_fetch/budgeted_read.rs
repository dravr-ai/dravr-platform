// ABOUTME: A live activity-list read that answers its turn inside the refresh budget
// ABOUTME: Past the budget the cached window answers and the read runs on as a durable job, so a slow scrape never fails a call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_database::repositories::BackfillJobKind;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::{ai_scope, CoreFitnessProvider};
use tokio::sync::oneshot::error::RecvError;
use tokio::time::{timeout, timeout_at, Instant};
use tracing::{info, warn};
use uuid::Uuid;

use super::pushed::serving_slug;
use super::sync_verdict::{HeadRead, HeadVerdict, LiveRead};
use super::{
    read_cached_page, read_live_window, serve_after_failed_read, serve_flagged_session,
    serve_stale_activities, stale_head_params, FailedRead,
};
use crate::activity_backfill::live_read::{
    start_live_read, Authenticated, JoinedRead, LiveReadOutput, LiveReadStart,
};
use crate::activity_backfill::{provider_tenant_id_str, ActivityBackfillJob};
use crate::context::TurnAsk;
use crate::runtime::ToolRuntime;

/// A live read a turn asks for, owned so the read can outlive the turn.
#[derive(Debug)]
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
    /// The conversation that asked, when a chat turn did: a read the turn
    /// stops waiting for posts its answer there once it lands.
    pub conversation_id: Option<String>,
    /// The turn and the athlete's message that asked, when a chat turn did:
    /// the follow-up answers that message only while it is the newest.
    pub ask: Option<TurnAsk>,
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
    /// The read outlived the budget, or a live read of the provider was
    /// already running, and the provider's own cached window answered. The
    /// read runs on as a job, and its write-through lands for the next turn.
    Cached(Vec<Activity>),
    /// A live read of the provider that was already running here landed over
    /// this window: the rows it wrote through answer, as current as the
    /// turn's own read would be, and nothing is owed.
    Landed(Vec<Activity>),
}

impl fmt::Debug for BudgetedRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Answered { read, provider } => f
                .debug_struct("Answered")
                .field("read", read)
                .field("provider", &provider.as_ref().map(|p| p.name()))
                .finish(),
            Self::Cached(activities) => f.debug_tuple("Cached").field(activities).finish(),
            Self::Landed(activities) => f.debug_tuple("Landed").field(activities).finish(),
        }
    }
}

/// Read a provider's window live, waiting at most `budget` for it.
///
/// A healthy scrape takes about two minutes, and the chat tool loop gives a
/// tool call ninety seconds: an inline read of a scraped provider could fail
/// the whole `get_activities` call on a provider that was working. So the
/// read is a [`BackfillJobKind::LiveRead`] job, recorded before it runs, and
/// the turn waits for its first attempt. Past the budget the turn is answered
/// from the provider's cached window, which the caller labels stale, while
/// the read goes on; it is judged, recorded and written through exactly as an
/// inline read would be, and the conversation that asked gets a follow-up
/// once it lands — or, when the athlete has asked something newer since, a
/// note that its data landed. With nothing cached there is nothing better to
/// say, so the turn keeps waiting for the read.
///
/// The fallback to the grant behind the athlete's session runs inside the
/// read ([`read_live_window`], carnet#1494), so it shares the read's budget:
/// past it, the cached window answers and a session that then fails falls
/// back in the job's read. A turn never reads both backends inline past its
/// budget.
///
/// A read of the provider already running here takes this ask over instead,
/// and the turn answers from the cache at once — or, when that read landed
/// over this window, with what it wrote, as current.
///
/// The read runs unfiltered, as a cache write does, on another task than the
/// turn's: what it answers is filtered here, back in the turn.
pub async fn read_live_window_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    provider: Box<dyn CoreFitnessProvider>,
    read: OwnedLiveRead,
    budget: Duration,
) -> BudgetedRead {
    // A push-only provider has no live list: its window is its pushed store,
    // read at once, with nothing to wait for or to owe.
    if runtime.provider_registry().is_push_only(provider.name()) {
        let tenant = read.tenant_id.to_string();
        let live = LiveRead {
            provider_slug: &read.provider_slug,
            user_id: read.user_id,
            tenant_id: Some(&tenant),
            params: &read.params,
            attempt_started_at: read.attempt_started_at,
        };
        let head = read_live_window(runtime, provider.as_ref(), live, Some(read.tenant_id)).await;
        return answered(
            runtime,
            Ok(LiveReadOutput {
                read: head,
                provider: Some(provider),
            }),
        );
    }
    let (provider_slug, user_id, tenant_id, params) = (
        read.provider_slug.clone(),
        read.user_id,
        read.tenant_id,
        read.params.clone(),
    );
    let authenticated = Authenticated {
        provider,
        attempt_started_at: read.attempt_started_at,
    };
    let deadline = Instant::now() + budget;
    let job = live_read_job(runtime, read);
    let mut answer = match start_live_read(job, Some(authenticated)).await {
        LiveReadStart::Started(answer) => answer,
        LiveReadStart::Joined(joined) => {
            return answer_joined(
                runtime,
                &joined,
                &provider_slug,
                user_id,
                tenant_id,
                &params,
            )
            .await;
        }
    };
    if let Ok(landed) = timeout_at(deadline, &mut answer).await {
        return answered(runtime, landed);
    }
    if let Some(cached) =
        serve_stale_activities(runtime, &provider_slug, user_id, tenant_id, &params).await
    {
        // Closed before it is read, so an answer that lands from here on is
        // the follow-up's to deliver — and one that landed already is
        // served rather than the cache.
        answer.close();
        if let Ok(landed) = answer.try_recv() {
            return answered(runtime, Ok(landed));
        }
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            budget_secs = budget.as_secs(),
            "live activity read outlived the turn's budget; answering from the cached window while it finishes"
        );
        return BudgetedRead::Cached(cached);
    }
    answered(runtime, answer.await)
}

/// Answer an ask that joined a live read of the pair already running.
///
/// A read that landed over this window and is settling — its follow-up's
/// re-ask of the question lands here — answers with what it wrote, as
/// current. Otherwise the cached window answers at once, and the running
/// read owes this ask its follow-up.
async fn answer_joined(
    runtime: &Arc<dyn ToolRuntime>,
    joined: &JoinedRead,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    params: &ActivityQueryParams,
) -> BudgetedRead {
    if joined.landed_over_its_window() {
        let rows = read_cached_page(runtime, provider_slug, user_id, tenant_id, params).await;
        return BudgetedRead::Landed(rows.unwrap_or_default());
    }
    info!(
        user_id = %user_id,
        provider = %provider_slug,
        "a live activity read of this provider is already running; answering from the cached window"
    );
    joined.owe_follow_up().await;
    let cached = serve_stale_activities(runtime, provider_slug, user_id, tenant_id, params).await;
    BudgetedRead::Cached(cached.unwrap_or_default())
}

/// The job a live read runs as.
fn live_read_job(runtime: &Arc<dyn ToolRuntime>, read: OwnedLiveRead) -> ActivityBackfillJob {
    let (turn_id, asked_message_id) = read
        .ask
        .map_or((None, None), |ask| (Some(ask.turn_id), ask.user_message_id));
    ActivityBackfillJob {
        resources: Arc::clone(runtime),
        user_id: read.user_id,
        tenant_id: read.tenant_id,
        tenant_id_str: provider_tenant_id_str(read.tenant_id),
        provider_name: read.provider_slug,
        query_params: read.params,
        pierre_conversation_id: read.conversation_id,
        kind: BackfillJobKind::LiveRead,
        turn_id,
        asked_message_id,
    }
}

fn answered(
    runtime: &Arc<dyn ToolRuntime>,
    landed: Result<LiveReadOutput, RecvError>,
) -> BudgetedRead {
    match landed {
        Ok(LiveReadOutput { read, provider }) => BudgetedRead::Answered {
            read: read.map(|mut head| {
                head.activities = ai_scope::filter_activities(
                    runtime.provider_registry().as_ref(),
                    head.activities,
                );
                head
            }),
            provider,
        },
        Err(RecvError { .. }) => BudgetedRead::Answered {
            read: Err(AppError::internal(
                "the live activity read ended without answering",
            )),
            provider: None,
        },
    }
}

/// Re-read a provider's recent head, waiting at most `budget` for it.
///
/// The head top-up of a covered historical window ([`super::refresh_stale_head`])
/// awaited [`super::refresh_head`] inline, so a slow scrape held the whole
/// call. It is a [`BackfillJobKind::LiveRead`] job no conversation waits on:
/// past the budget the read runs on and its write-through lands for the next
/// turn, silently; `None` tells the caller the head it serves is behind. A
/// read that failed is answered from the cache, as [`super::refresh_head`]'s
/// is, and a flagged scrape session is not read at all. Run unfiltered and
/// filtered here, for the reason [`read_live_window_within_budget`] gives.
pub async fn refresh_head_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    budget: Duration,
) -> Option<Vec<Activity>> {
    let attempt_started_at = Utc::now();
    let provider_slug = &serving_slug(runtime, provider_slug, user_id, Some(tenant_id)).await;
    let params = stale_head_params();
    if let Some(served) =
        serve_flagged_session(runtime, provider_slug, user_id, Some(tenant_id), &params).await
    {
        return served;
    }
    let read = OwnedLiveRead {
        provider_slug: provider_slug.clone(),
        user_id,
        tenant_id,
        params: params.clone(),
        attempt_started_at,
        conversation_id: None,
        ask: None,
    };
    let landed = head_within_budget(runtime, read, budget).await?;
    let failed = FailedRead {
        provider_slug,
        user_id,
        tenant: Some(tenant_id),
        params: &params,
        attempt_started_at,
    };
    let rows = served_head(runtime, failed, landed.read).await?;
    Some(ai_scope::filter_activities(
        runtime.provider_registry().as_ref(),
        rows,
    ))
}

/// Start a stale head's read and wait at most `budget` for its first
/// attempt; `None` when it is still running, or a read of the provider
/// already was.
async fn head_within_budget(
    runtime: &Arc<dyn ToolRuntime>,
    read: OwnedLiveRead,
    budget: Duration,
) -> Option<LiveReadOutput> {
    let (user_id, provider_slug) = (read.user_id, read.provider_slug.clone());
    let LiveReadStart::Started(mut answer) =
        start_live_read(live_read_job(runtime, read), None).await
    else {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            "a live activity read of this provider is already running; serving the cached head"
        );
        return None;
    };
    match timeout(budget, &mut answer).await {
        Ok(Ok(landed)) => Some(landed),
        Ok(Err(RecvError { .. })) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                "stale activity head refresh ended without answering"
            );
            None
        }
        Err(_) => {
            info!(
                user_id = %user_id,
                provider = %provider_slug,
                budget_secs = budget.as_secs(),
                "stale activity head did not refresh within the turn's budget; serving the cached head while it finishes"
            );
            None
        }
    }
}

/// The head a refresh's read answers with: its own rows when the provider
/// vouched for the list, else the cached head — the connection that refused
/// flagged first, as for every failed head read.
async fn served_head(
    runtime: &Arc<dyn ToolRuntime>,
    failed: FailedRead<'_>,
    read: AppResult<HeadRead>,
) -> Option<Vec<Activity>> {
    match read {
        Ok(HeadRead {
            activities,
            verdict: HeadVerdict::Complete | HeadVerdict::Incomplete,
            ..
        }) => Some(activities),
        Ok(HeadRead {
            verdict: HeadVerdict::EmptyOverCached { .. },
            ..
        }) => {
            let tenant = failed.tenant?;
            serve_stale_activities(
                runtime,
                failed.provider_slug,
                failed.user_id,
                tenant,
                failed.params,
            )
            .await
        }
        Err(e) => serve_after_failed_read(runtime, failed, &e).await,
    }
}
