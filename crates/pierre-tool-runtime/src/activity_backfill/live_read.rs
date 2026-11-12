// ABOUTME: A live activity read as a durable job: the turn waits for its first attempt, the ledger owes the rest
// ABOUTME: A read that outran its turn answers the newest ask once it lands, or notes the data landed when the athlete moved on
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A [`BackfillJobKind::LiveRead`].
//!
//! A chat turn's live list read is recorded as a job row before it runs,
//! like every backfill: an instance that dies under it leaves the row for the
//! resume sweep, and the follow-up the turn promised is not lost with it. The
//! turn that asked hands the job the provider it authenticated and waits for
//! the first attempt's answer, up to its budget
//! ([`crate::activity_fetch::budgeted_read`]); past it the turn answers from
//! the cache and the read is owed to the conversation. Only then does the
//! landed read post a follow-up — to the ask the row is bound to when it
//! finishes, which a newer ask rebinds.

use std::fmt;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{BackfillJobKind, JobLease, LiveReadAsk};
use pierre_providers::ai_scope;
use pierre_providers::CoreFitnessProvider;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{info, instrument, warn};

use super::{
    enqueue, job_row, keep_backfill_lease, notify_backfill_reauth, remove_backfill_job,
    settle_backfill_job, ActivityBackfillJob, BackfillRunOutcome,
};
use crate::activity_fetch::read_live_window;
use crate::activity_fetch::sync_verdict::{HeadRead, HeadVerdict, LiveRead};
use crate::protocol::auth::AuthService;
use crate::protocol::types::provider_auth_failure;
use crate::revalidation::revalidation_timeout;
use crate::runtime::BackfillCompletion;

/// What one attempt of a live read answered, for the turn waiting on it.
pub struct LiveReadOutput {
    /// What [`read_live_window`] returned, unfiltered: the turn filters it
    /// for the model it answers.
    pub read: AppResult<HeadRead>,
    /// The authenticated provider, when the attempt still holds one.
    pub provider: Option<Box<dyn CoreFitnessProvider>>,
}

impl fmt::Debug for LiveReadOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveReadOutput")
            .field("read", &self.read)
            .field("provider", &self.provider.as_ref().map(|p| p.name()))
            .finish()
    }
}

/// A provider the asking turn already authenticated, and when it began.
///
/// The attempt began before the credential was read, so a reconnect that
/// lands while the read is in flight is newer than any refusal it reports.
pub struct Authenticated {
    /// The provider.
    pub provider: Box<dyn CoreFitnessProvider>,
    /// When the turn's attempt began.
    pub attempt_started_at: DateTime<Utc>,
}

impl fmt::Debug for Authenticated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Authenticated")
            .field("provider", &self.provider.name())
            .field("attempt_started_at", &self.attempt_started_at)
            .finish()
    }
}

/// The waiting turn's side of a live read's first attempt.
pub(super) struct LiveReadHandoff {
    authenticated: Option<Authenticated>,
    reply: oneshot::Sender<LiveReadOutput>,
}

/// How asking for a live read went.
#[derive(Debug)]
pub enum LiveReadStart {
    /// This ask started the read: its first attempt answers on the receiver.
    /// Dropping or closing the receiver before the answer tells the read the
    /// turn stopped waiting, and the read then owes the conversation a
    /// follow-up.
    Started(oneshot::Receiver<LiveReadOutput>),
    /// A live read of the pair is already owed. An ask with a conversation
    /// rebound it, so the follow-up answers this ask once it lands.
    Joined,
}

/// Record `job` (a [`BackfillJobKind::LiveRead`]) and start its first
/// attempt with the provider the turn already holds, or with one the
/// attempt authenticates when `authenticated` is `None`.
pub async fn start_live_read(
    job: ActivityBackfillJob,
    authenticated: Option<Authenticated>,
) -> LiveReadStart {
    let (reply, answer) = oneshot::channel();
    let handoff = LiveReadHandoff {
        authenticated,
        reply,
    };
    if enqueue(job, Some(handoff)).await {
        LiveReadStart::Started(answer)
    } else {
        LiveReadStart::Joined
    }
}

/// How an attempt ended, for its row.
enum Attempt {
    /// The provider answered a list: this many activities.
    Landed(usize),
    /// The session lapsed; the athlete must reconnect.
    AuthRequired,
    /// Anything else, an empty answer the cache contradicts included.
    Failed,
}

impl Attempt {
    fn of(read: &AppResult<HeadRead>) -> Self {
        match read {
            Ok(HeadRead {
                activities,
                verdict: HeadVerdict::Complete | HeadVerdict::Incomplete,
            }) => Self::Landed(activities.len()),
            Err(e) if e.provider_auth_required_provider().is_some() => Self::AuthRequired,
            Ok(HeadRead {
                verdict: HeadVerdict::EmptyOverCached { .. },
                ..
            })
            | Err(_) => Self::Failed,
        }
    }
}

/// Run one attempt of a live read and settle its row.
///
/// The attempt answers the turn waiting on it, when one is. A turn that got
/// the answer is owed nothing more; a turn that stopped waiting — and every
/// attempt the resume sweep runs, which no turn waits on — is answered by the
/// follow-up once the read lands, and a failure leaves the row for the next
/// attempt. A newer ask that rebound the row is owed its follow-up either way.
#[instrument(
    skip_all,
    name = "live_read",
    fields(user_id = %job.user_id, tenant_id = %job.tenant_id, provider = %job.provider_name)
)]
pub(super) async fn run_live_read(
    job: &ActivityBackfillJob,
    recorded: Option<(JobLease, i64)>,
    handoff: Option<LiveReadHandoff>,
) {
    let renewal = recorded
        .as_ref()
        .map(|(lease, _)| keep_backfill_lease(job, lease));
    let (authenticated, reply) = match handoff {
        Some(handoff) => (handoff.authenticated, Some(handoff.reply)),
        None => (None, None),
    };
    // Taken before the session is read, whoever authenticated it: a reconnect
    // that lands during a long scrape is newer than the failure it reports.
    let attempt_started_at = authenticated
        .as_ref()
        .map_or_else(Utc::now, |authenticated| authenticated.attempt_started_at);
    let output = read_once(job, authenticated, attempt_started_at).await;
    let attempt = Attempt::of(&output.read);
    let delivered = reply.is_some_and(|reply| reply.send(output).is_ok());
    match (attempt, recorded) {
        (Attempt::Landed(count), Some((lease, _))) => {
            if let Some(ask) = finish(job, &lease).await {
                if !delivered || rebound(job, &ask) {
                    follow_up(job, &ask, count).await;
                }
            }
        }
        (Attempt::Landed(count), None) if !delivered => {
            follow_up(job, &own_ask(job), count).await;
        }
        (Attempt::AuthRequired, recorded) => {
            if !delivered {
                notify_backfill_reauth(job, attempt_started_at).await;
            }
            if let Some((lease, _)) = recorded {
                remove_backfill_job(job, &lease, "auth_required").await;
            }
        }
        (Attempt::Failed, Some((lease, attempts))) if !delivered => {
            settle_backfill_job(job, &lease, attempts, &BackfillRunOutcome::Failed).await;
        }
        (Attempt::Failed, Some((lease, attempts))) => {
            // The turn heard of the failure itself; only a newer ask the
            // row was rebound to is still owed, and it keeps the row.
            if let Some(ask) = finish(job, &lease).await {
                if rebound(job, &ask) {
                    owe(job, ask, attempts).await;
                }
            }
        }
        (Attempt::Landed(_) | Attempt::Failed, None) => {}
    }
    drop(renewal);
}

/// Read the job's window once, through `authenticated` or a provider this
/// attempt authenticates, capped at [`revalidation_timeout`]. Run unfiltered,
/// as a cache write is: the turn filters what it answers.
async fn read_once(
    job: &ActivityBackfillJob,
    authenticated: Option<Authenticated>,
    attempt_started_at: DateTime<Utc>,
) -> LiveReadOutput {
    let Authenticated {
        provider,
        attempt_started_at,
    } = match authenticated {
        Some(authenticated) => authenticated,
        None => match authenticate(job, attempt_started_at).await {
            Ok(authenticated) => authenticated,
            Err(e) => {
                return LiveReadOutput {
                    read: Err(e),
                    provider: None,
                }
            }
        },
    };
    let cap = revalidation_timeout(&job.resources.config().timeouts);
    let tenant = job.tenant_id.to_string();
    let live = LiveRead {
        provider_slug: &job.provider_name,
        user_id: job.user_id,
        tenant_id: Some(&tenant),
        params: &job.query_params,
        attempt_started_at,
    };
    let read = ai_scope::unfiltered(read_live_window(
        &job.resources,
        provider.as_ref(),
        live,
        Some(job.tenant_id),
    ));
    timeout(cap, read).await.map_or_else(
        |_| {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                timeout_secs = cap.as_secs(),
                "live activity read timed out"
            );
            LiveReadOutput {
                read: Err(AppError::external_service(
                    "activity provider",
                    format!("the live read did not answer within {}s", cap.as_secs()),
                )),
                provider: None,
            }
        },
        |read| LiveReadOutput {
            read,
            provider: Some(provider),
        },
    )
}

/// Authenticate the job's provider for an attempt no turn handed one to.
async fn authenticate(
    job: &ActivityBackfillJob,
    attempt_started_at: DateTime<Utc>,
) -> AppResult<Authenticated> {
    let provider = AuthService::new(job.resources.clone())
        .create_authenticated_provider(
            &job.provider_name,
            job.user_id,
            job.tenant_id_str.as_deref(),
        )
        .await
        .map_err(|response| provider_auth_failure(&job.provider_name, &response))?;
    Ok(Authenticated {
        provider,
        attempt_started_at,
    })
}

/// Delete the landed read's row and read back the ask it was bound to;
/// `None` when the row is no longer this run's, whose holder answers it.
async fn finish(job: &ActivityBackfillJob, lease: &JobLease) -> Option<LiveReadAsk> {
    match job
        .resources
        .repos()
        .activity_backfill_jobs
        .finish_live_read(lease)
        .await
    {
        Ok(Some(ask)) => Some(ask),
        Ok(None) => {
            info!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                row_id = %lease,
                "Live read: the job row is no longer this run's; its holder answers it"
            );
            None
        }
        Err(e) => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                row_id = %lease,
                error = %e,
                "Live read: the job row could not be finished; answering the ask that started it"
            );
            Some(own_ask(job))
        }
    }
}

/// The ask the run itself was started for.
fn own_ask(job: &ActivityBackfillJob) -> LiveReadAsk {
    LiveReadAsk {
        conversation_id: job.pierre_conversation_id.clone(),
        turn_id: job.turn_id.clone(),
        asked_message_id: job.asked_message_id.clone(),
    }
}

/// Whether a newer ask than the one that started the run took the row over.
fn rebound(job: &ActivityBackfillJob, ask: &LiveReadAsk) -> bool {
    let bound = (
        ask.conversation_id.as_deref(),
        ask.asked_message_id.as_deref(),
    );
    let started = (
        job.pierre_conversation_id.as_deref(),
        job.asked_message_id.as_deref(),
    );
    bound != started
}

/// Put the read back on file for a newer ask the failed attempt's turn did
/// not make, lapsed so the resume sweep takes it: that ask was answered from
/// the cache and is still owed its follow-up.
async fn owe(job: &ActivityBackfillJob, ask: LiveReadAsk, attempts: i64) {
    let mut row = job_row(job);
    row.conversation_id = ask.conversation_id;
    row.turn_id = ask.turn_id;
    row.asked_message_id = ask.asked_message_id;
    row.leased_until_ms = 0;
    row.lease_token = None;
    row.attempts = attempts;
    if let Err(e) = job
        .resources
        .repos()
        .activity_backfill_jobs
        .record_backfill_job(&row)
        .await
    {
        warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            error = %e,
            "Live read: a newer ask's read could not be put back on file"
        );
    }
}

/// Answer the conversation `ask` names, now that the read landed: a re-ask of
/// the athlete's question, or a note that its data landed when they have
/// asked something newer since — the notifier tells which.
async fn follow_up(job: &ActivityBackfillJob, ask: &LiveReadAsk, activity_count: usize) {
    let (Some(conversation_id), Some(notifier)) = (
        ask.conversation_id.as_deref(),
        job.resources.backfill_notifier(),
    ) else {
        return;
    };
    info!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        turn_id = ?ask.turn_id,
        activities = activity_count,
        "Live read landed after its turn; answering the conversation that asked"
    );
    notifier
        .push_backfill_complete(BackfillCompletion {
            user_id: job.user_id,
            tenant_id: job.tenant_id,
            pierre_conversation_id: conversation_id,
            provider: &job.provider_name,
            after_ts: job.query_params.after.unwrap_or(0),
            activity_count,
            kind: BackfillJobKind::LiveRead,
            asked_message_id: ask.asked_message_id.as_deref(),
        })
        .await;
}
