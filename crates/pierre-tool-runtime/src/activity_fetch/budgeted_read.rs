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
use tokio::sync::oneshot::{self, error::RecvError};
use tokio::time::{self, Instant};
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
    /// No read answers this ask in its turn or owes it a follow-up — the
    /// pair's read is another tenant's, or the ledger cannot say whether one
    /// runs — and none could be started for it: the provider's cached window
    /// answers, as a read that did not run, never as one still running.
    Unread(Vec<Activity>),
    /// A live read of the provider that was already running here landed over
    /// this window: the rows it wrote through answer, as current as the
    /// turn's own read would be, and nothing is owed.
    Landed {
        /// The rows the read wrote through.
        activities: Vec<Activity>,
        /// The grant that answered in the session's place, when one did:
        /// the rows are its.
        served_by_grant: Option<String>,
    },
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
            Self::Unread(activities) => f.debug_tuple("Unread").field(activities).finish(),
            Self::Landed {
                activities,
                served_by_grant,
            } => f
                .debug_struct("Landed")
                .field("activities", activities)
                .field("served_by_grant", served_by_grant)
                .finish(),
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
/// A read of the provider already running — here, or owed in the ledger —
/// is joined rather than read twice ([`wait_on_joined`]): with rows cached
/// they answer at once and the running read owes this ask its follow-up;
/// with nothing cached the turn waits for that read, inside the same budget.
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
            let window = Window {
                provider_slug: &provider_slug,
                user_id,
                tenant_id,
                params: &params,
            };
            match wait_on_joined(runtime, joined, &window, deadline).await {
                AfterJoin::Answered(answered) => return answered,
                AfterJoin::Started(answer) => answer,
            }
        }
    };
    if let Ok(landed) = time::timeout_at(deadline, &mut answer).await {
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

/// How often a turn that joined a running read looks at it again.
const JOINED_READ_POLL: Duration = Duration::from_millis(250);

/// The window a budgeted read answers.
#[derive(Clone, Copy)]
struct Window<'read> {
    provider_slug: &'read str,
    user_id: Uuid,
    tenant_id: TenantId,
    params: &'read ActivityQueryParams,
}

/// How waiting on a joined read ended.
enum AfterJoin {
    /// The turn's answer.
    Answered(BudgetedRead),
    /// The joined read ended without answering this window, and this ask
    /// started its own read.
    Started(oneshot::Receiver<LiveReadOutput>),
}

/// Wait, until `deadline`, on the live read of the pair this ask joined.
///
/// - The read gave up here, its last attempt failed: the cached window
///   answers at once, as a read that did not run ([`BudgetedRead::Unread`]).
///   The follow-up that read owed re-asks the question here, and must not
///   read the provider again.
/// - The read landed here over this window: its rows answer as current
///   ([`BudgetedRead::Landed`]). The follow-up of a read that outlived its
///   turn re-asks the question while the run still holds the pair, and
///   lands here.
/// - Rows are cached and the joined read still owes a follow-up: they
///   answer at once, and that read owes this ask its follow-up.
/// - The joined read is not running — it landed over a narrower window, it
///   ended, or its row stands owed with no attempt holding it: this ask
///   starts its own read, which takes over an idle row.
/// - Otherwise the turn waits for the read, as it would for its own, until
///   the deadline ([`past_deadline`]).
async fn wait_on_joined(
    runtime: &Arc<dyn ToolRuntime>,
    mut joined: Box<JoinedRead>,
    window: &Window<'_>,
    deadline: Instant,
) -> AfterJoin {
    let Window {
        provider_slug,
        user_id,
        ..
    } = *window;
    info!(
        user_id = %user_id,
        provider = %provider_slug,
        "a live activity read of this provider is already running; joining it"
    );
    loop {
        if let Some(answer) = answer_beside(runtime, &joined, window).await {
            return AfterJoin::Answered(answer);
        }
        if Instant::now() >= deadline {
            info!(
                user_id = %user_id,
                provider = %provider_slug,
                "the joined live activity read did not land within the turn's budget"
            );
            return past_deadline(runtime, joined, window).await;
        }
        // A read joined again is looked at from the top: one that landed
        // over this window answers it.
        if !joined.running().await {
            match (*joined).restart().await {
                LiveReadStart::Started(answer) => return AfterJoin::Started(answer),
                LiveReadStart::Joined(again) => joined = again,
            }
        }
        time::sleep_until(deadline.min(Instant::now() + JOINED_READ_POLL)).await;
    }
}

/// The turn's budget is spent, and the read joined has not answered it.
///
/// - The read joined owes this ask its follow-up: the cached window
///   answers, empty when nothing is cached, as a read still running.
/// - It owes this ask nothing (it landed, ended, or its row is not this
///   ask's): the ask asks again, once. Its own read starts, and answers as
///   it would have alone; or a read that landed over this window meanwhile
///   answers as current; or the read joined again owes it the follow-up.
///   Failing all three, the cached window answers as a read that did not
///   run ([`BudgetedRead::Unread`]): no follow-up is coming.
async fn past_deadline(
    runtime: &Arc<dyn ToolRuntime>,
    joined: Box<JoinedRead>,
    window: &Window<'_>,
) -> AfterJoin {
    let Window {
        provider_slug,
        user_id,
        tenant_id,
        params,
    } = *window;
    let owed = joined.owe_follow_up().await;
    let joined = if owed {
        joined
    } else {
        match (*joined).restart().await {
            LiveReadStart::Started(answer) => return AfterJoin::Started(answer),
            LiveReadStart::Joined(again) => {
                if let Some(landed) = landed_answer(runtime, &again, window).await {
                    return AfterJoin::Answered(landed);
                }
                again
            }
        }
    };
    let cached = serve_stale_activities(runtime, provider_slug, user_id, tenant_id, params)
        .await
        .unwrap_or_default();
    if owed || joined.owe_follow_up().await {
        AfterJoin::Answered(BudgetedRead::Cached(cached))
    } else {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            "no live activity read answers or owes this ask; answering the cached window as unread"
        );
        AfterJoin::Answered(BudgetedRead::Unread(cached))
    }
}

/// The rows the joined read landed over this window, as current; `None`
/// while it has not.
async fn landed_answer(
    runtime: &Arc<dyn ToolRuntime>,
    joined: &JoinedRead,
    window: &Window<'_>,
) -> Option<BudgetedRead> {
    let landed = joined.landed_over_its_window()?;
    let filed_under = landed.filed_under(window.provider_slug);
    let rows = read_cached_page(
        runtime,
        filed_under,
        window.user_id,
        window.tenant_id,
        window.params,
    )
    .await;
    Some(BudgetedRead::Landed {
        activities: rows.unwrap_or_default(),
        served_by_grant: landed.served_by_grant,
    })
}

/// What the joined read answers this ask with now: the cached rows as
/// unread when it gave up, the rows it landed over this window, or the
/// cached rows while it still owes this ask a follow-up.
async fn answer_beside(
    runtime: &Arc<dyn ToolRuntime>,
    joined: &JoinedRead,
    window: &Window<'_>,
) -> Option<BudgetedRead> {
    let Window {
        provider_slug,
        user_id,
        tenant_id,
        params,
    } = *window;
    if joined.gave_up() {
        info!(
            user_id = %user_id,
            provider = %provider_slug,
            "the live activity read of this provider gave up; answering the cached window as unread"
        );
        let cached = serve_stale_activities(runtime, provider_slug, user_id, tenant_id, params)
            .await
            .unwrap_or_default();
        return Some(BudgetedRead::Unread(cached));
    }
    if let Some(landed) = landed_answer(runtime, joined, window).await {
        return Some(landed);
    }
    let cached = serve_stale_activities(runtime, provider_slug, user_id, tenant_id, params).await?;
    joined
        .owe_follow_up()
        .await
        .then_some(BudgetedRead::Cached(cached))
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
    match time::timeout(budget, &mut answer).await {
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
