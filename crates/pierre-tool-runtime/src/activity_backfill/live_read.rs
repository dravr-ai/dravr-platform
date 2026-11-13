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
//!
//! An ask that finds a read of the pair already running joins it
//! ([`JoinedRead`]) and waits on it rather than reading twice. A read that
//! landed here over the ask's window answers it as current: that is how its
//! own follow-up's re-ask of the question, asked while the run still holds
//! the pair, reads the rows the run just wrote.
//!
//! Only a read that is actually running is joined. One that landed here
//! over a narrower window than the ask's holds no provider read any more,
//! so the ask reads for itself ([`succeeds`]). A failed attempt gives its
//! lease back: its row stays owed for the sweep's bounded retries, but the
//! next ask finds it idle, takes it over and reads it in its own turn
//! ([`take_over_idle`]) — that ask's answer stands as its owed follow-up.
//!
//! A read whose last attempt fails still owes the ask it is bound to its
//! follow-up ([`give_up`]). Its run then reads [`InFlight::GaveUp`]: the
//! follow-up's re-ask joins it and is answered from what is stored, the
//! provider unavailable, never by another read that would fail and follow
//! up again.

use std::fmt;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobKind, JobLease, JobLeaseState, LiveReadAsk,
};
use pierre_providers::ai_scope;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::CoreFitnessProvider;
use tokio::sync::{oneshot, watch};
use tokio::time;
use tracing::{info, instrument, warn};

use super::{
    enqueue, job_row, keep_backfill_lease, lease_ms, mark_in_flight, notify_backfill_reauth,
    now_ms, rebind_live_read, remove_backfill_job, ActivityBackfillJob, Enqueued, InFlight,
    InFlightClaim, MAX_BACKFILL_ATTEMPTS,
};
use crate::activity_fetch::read_live_window;
use crate::activity_fetch::sync_verdict::{HeadRead, HeadVerdict, LiveRead};
use crate::protocol::auth::AuthService;
use crate::protocol::types::provider_auth_failure;
use crate::revalidation::revalidation_timeout;
use crate::runtime::BackfillCompletion;

/// What one attempt of a live read answered, for the turn waiting on it.
pub(crate) struct LiveReadOutput {
    /// What [`read_live_window`] returned, unfiltered: the turn filters it
    /// for the model it answers.
    pub(crate) read: AppResult<HeadRead>,
    /// The authenticated provider, when the attempt still holds one.
    pub(crate) provider: Option<Box<dyn CoreFitnessProvider>>,
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
pub(crate) struct Authenticated {
    /// The provider.
    pub(crate) provider: Box<dyn CoreFitnessProvider>,
    /// When the turn's attempt began.
    pub(crate) attempt_started_at: DateTime<Utc>,
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
pub(crate) enum LiveReadStart {
    /// This ask started the read: its first attempt answers on the receiver.
    /// Dropping or closing the receiver before the answer tells the read the
    /// turn stopped waiting, and the read then owes the conversation a
    /// follow-up.
    Started(oneshot::Receiver<LiveReadOutput>),
    /// A live read of the pair already runs here or is owed: this ask joined
    /// it, and waits on it through the [`JoinedRead`].
    Joined(Box<JoinedRead>),
}

/// Record `job` (a [`BackfillJobKind::LiveRead`]) and start its first
/// attempt with the provider the turn already holds, or with one the
/// attempt authenticates when `authenticated` is `None`.
pub(crate) async fn start_live_read(
    job: ActivityBackfillJob,
    authenticated: Option<Authenticated>,
) -> LiveReadStart {
    let (reply, answer) = oneshot::channel();
    let handoff = LiveReadHandoff {
        authenticated,
        reply,
    };
    let (job, handoff, here) = match enqueue(job, Some(handoff)).await {
        Enqueued::Started => return LiveReadStart::Started(answer),
        Enqueued::InFlightHere(job, handoff, run) => (job, handoff, Some(run)),
        Enqueued::Owed(job, handoff) => (job, handoff, None),
    };
    // A read owed elsewhere answers this ask only when its row is this
    // tenant's: recording the ask rebound that row to it. Another tenant's
    // row of the pair blocks the ask without ever answering it.
    let bound_elsewhere =
        here.is_none() && owed_lease(&job).await.is_ok_and(|lease| lease.is_some());
    LiveReadStart::Joined(Box::new(JoinedRead {
        job,
        authenticated: handoff.and_then(|handoff| handoff.authenticated),
        here,
        bound_elsewhere,
    }))
}

/// An ask that found a live read of its pair already running, here or owed
/// in the ledger, and was not started.
///
/// It waits on that read instead of reading twice. The read is rebound to
/// this ask only when the turn stops waiting ([`Self::owe_follow_up`]): a
/// turn that waited for the read's rows and answered with them is owed no
/// follow-up. An ask joined to a read owed elsewhere was rebound to it as it
/// was recorded.
pub(crate) struct JoinedRead {
    job: ActivityBackfillJob,
    authenticated: Option<Authenticated>,
    /// The run joined, when it runs in this process.
    here: Option<watch::Receiver<InFlight>>,
    /// Whether the read owed elsewhere that this ask joined is a row of its
    /// own tenant, which recording the ask rebound to it.
    bound_elsewhere: bool,
}

impl fmt::Debug for JoinedRead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinedRead")
            .field("job", &self.job)
            .field("authenticated", &self.authenticated)
            .field("here", &self.here.as_ref().map(|run| run.borrow().clone()))
            .field("bound_elsewhere", &self.bound_elsewhere)
            .finish()
    }
}

impl JoinedRead {
    /// Whether the read joined landed here, reading a window that holds this
    /// ask's: the rows it wrote through are then as current as a read of
    /// this ask's own, and the answer says where they are filed. Still
    /// answered once the run has ended.
    #[must_use]
    pub(crate) fn landed_over_its_window(&self) -> Option<LandedOver> {
        let run = self.here.as_ref()?;
        let state = run.borrow();
        match &*state {
            InFlight::Landed {
                params,
                count,
                served_by_grant,
            } if covers(params, *count, &self.job.query_params) => Some(LandedOver {
                served_by_grant: served_by_grant.clone(),
            }),
            InFlight::Landed { .. } | InFlight::Running | InFlight::GaveUp => None,
        }
    }

    /// Whether the read joined here spent its last attempt
    /// ([`InFlight::GaveUp`]): the ask is answered from what is stored, the
    /// provider unavailable, and reads nothing. Still answered once the run
    /// has ended.
    #[must_use]
    pub(crate) fn gave_up(&self) -> bool {
        self.here
            .as_ref()
            .is_some_and(|run| matches!(*run.borrow(), InFlight::GaveUp))
    }

    /// Whether the read joined still runs: the run here is still reading,
    /// or an attempt holds the lease of the read the ledger owes. A run here
    /// that landed reads nothing more, and an owed row no attempt holds is
    /// waiting for a retry, not running.
    pub(crate) async fn running(&self) -> bool {
        if let Some(run) = &self.here {
            if matches!(*run.borrow(), InFlight::Landed { .. } | InFlight::GaveUp) {
                return false;
            }
            if run.has_changed().is_ok() {
                return true;
            }
        }
        matches!(owed_lease(&self.job).await, Ok(Some(JobLeaseState::Leased)))
    }

    /// The turn stops waiting: the read joined owes this ask its follow-up
    /// once it lands. A read running here is rebound to it now; one owed
    /// elsewhere was rebound as this ask was recorded.
    ///
    /// `false` when no follow-up would come: the run here landed or ended,
    /// or its row is not this ask's to rebind; the read owed elsewhere is
    /// another tenant's, which never answers this ask, or stands idle, no
    /// attempt holding it; or the ledger cannot say, which reads as no read
    /// running, as [`Self::running`] reads it. The ask then reads for itself,
    /// or is answered as no read owes it, rather than be told a read is
    /// coming. A row of its own that is gone was finished by the read that
    /// landed it, whose follow-up answers the ask the row was bound to.
    pub(crate) async fn owe_follow_up(&self) -> bool {
        let Some(run) = &self.here else {
            return self.bound_elsewhere
                && matches!(
                    owed_lease(&self.job).await,
                    Ok(Some(JobLeaseState::Leased) | None)
                );
        };
        if matches!(*run.borrow(), InFlight::Landed { .. } | InFlight::GaveUp)
            || run.has_changed().is_err()
        {
            return false;
        }
        // An ask with no conversation is owed nothing a follow-up could post.
        self.job.pierre_conversation_id.is_none() || rebind_live_read(&self.job).await
    }

    /// Ask for the read again, now that the one joined has ended.
    pub(crate) async fn restart(self) -> LiveReadStart {
        start_live_read(self.job, self.authenticated).await
    }
}

/// Whether an attempt holds the lease of the live read the ledger owes for
/// `job`'s pair in its tenant; `Ok(None)` when none is owed there. A ledger
/// that cannot say is logged.
async fn owed_lease(job: &ActivityBackfillJob) -> AppResult<Option<JobLeaseState>> {
    job.resources
        .repos()
        .activity_backfill_jobs
        .backfill_job_lease_state(
            job.user_id,
            job.tenant_id,
            &job.provider_name,
            BackfillJobKind::LiveRead,
            now_ms(),
        )
        .await
        .inspect_err(|e| {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = %e,
                "Live read: the ledger could not say whether a read of the pair is running; reading it as not"
            );
        })
}

/// A joined read that landed over the ask's window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LandedOver {
    /// The grant that answered in the session's place, when one did: the
    /// rows are filed under it, not under the session the ask named.
    pub(crate) served_by_grant: Option<String>,
}

impl LandedOver {
    /// The cache key the landed rows are filed under, for an ask of
    /// `provider_slug`.
    #[must_use]
    pub(crate) fn filed_under<'a>(&'a self, provider_slug: &'a str) -> &'a str {
        self.served_by_grant.as_deref().unwrap_or(provider_slug)
    }
}

/// Whether `job`, a live read, succeeds the run of its pair in `run`: that
/// run landed over a window that does not hold `job`'s. It reads nothing
/// more — its row is finished, only its follow-up is left — so the ask reads
/// its own window rather than wait on a run that will never read it.
pub(super) fn succeeds(job: &ActivityBackfillJob, run: &InFlight) -> bool {
    job.kind == BackfillJobKind::LiveRead
        && matches!(run, InFlight::Landed { params, count, .. }
            if !covers(params, *count, &job.query_params))
}

/// Take over the live read owed for `job`'s pair when no attempt holds it,
/// leased under `row`: a failed attempt gave it back, or its instance died.
/// The ask reads it now instead of joining a read nothing runs; `None` for
/// any other kind, or when an attempt holds the row.
pub(super) async fn take_over_idle(
    job: &ActivityBackfillJob,
    row: &ActivityBackfillJobRow,
) -> Option<(JobLease, i64)> {
    if job.kind != BackfillJobKind::LiveRead {
        return None;
    }
    match job
        .resources
        .repos()
        .activity_backfill_jobs
        .take_over_idle_live_read(row)
        .await
    {
        Ok(Some(taken)) => {
            info!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                attempts = taken.1,
                "Live read: the read owed for the pair stood idle; this ask reads it"
            );
            Some(taken)
        }
        Ok(None) => None,
        Err(e) => {
            warn!(
                user_id = %job.user_id,
                provider = %job.provider_name,
                error = %e,
                "Live read: an idle read could not be taken over; joining it"
            );
            None
        }
    }
}

/// Whether a read of `read` that answered `landed` activities saw every
/// activity of `asked`: its window holds `asked`'s, it began at the newest,
/// and it either ran out of activities before its limit or reached as far
/// down as `asked` pages.
fn covers(read: &ActivityQueryParams, landed: usize, asked: &ActivityQueryParams) -> bool {
    let from = read
        .after
        .is_none_or(|after| asked.after.is_some_and(|from| from >= after));
    let to = read
        .before
        .is_none_or(|before| asked.before.is_some_and(|to| to <= before));
    let from_the_newest = read.offset.unwrap_or(0) == 0;
    let deep_enough = read.limit.is_none_or(|limit| {
        landed < limit
            || asked
                .limit
                .is_some_and(|rows| asked.offset.unwrap_or(0).saturating_add(rows) <= limit)
    });
    from && to && from_the_newest && deep_enough
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
                ..
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
/// follow-up once the read lands, and a failure leaves the row, idle, for the
/// next attempt. A newer ask that rebound the row is owed its follow-up
/// either way.
#[instrument(
    skip_all,
    name = "live_read",
    fields(user_id = %job.user_id, tenant_id = %job.tenant_id, provider = %job.provider_name)
)]
pub(super) async fn run_live_read(
    job: &ActivityBackfillJob,
    claim: &InFlightClaim,
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
    drop(renewal);
    let attempt = Attempt::of(&output.read);
    // A landed read finishes its row before it marks itself landed, and both
    // before anything is answered. From then on the run holds no read and no
    // row: its follow-up's re-ask of the question reads what landed, and an
    // ask its window does not cover reads for itself ([`succeeds`]) rather
    // than be promised a follow-up from a row that is gone.
    let bound = match (&attempt, &recorded) {
        (Attempt::Landed(_), Some((lease, _))) => finish(job, lease).await,
        _ => None,
    };
    if let (Attempt::Landed(count), Ok(head)) = (&attempt, &output.read) {
        let landed = InFlight::Landed {
            params: job.query_params.clone(),
            count: *count,
            served_by_grant: head.served_by_grant.clone(),
        };
        mark_in_flight(claim, landed);
    }
    let delivered = reply.is_some_and(|reply| reply.send(output).is_ok());
    let ended = Ended {
        bound,
        delivered,
        attempt_started_at,
    };
    settle(job, claim, attempt, recorded, ended).await;
}

/// How an attempt's answer went out, for its row.
struct Ended {
    /// The ask a landed read's finished row was bound to.
    bound: Option<LiveReadAsk>,
    /// Whether the turn that asked heard the answer itself.
    delivered: bool,
    /// When the attempt began.
    attempt_started_at: DateTime<Utc>,
}

/// Settle the row of an attempt that ended as `attempt`.
async fn settle(
    job: &ActivityBackfillJob,
    claim: &InFlightClaim,
    attempt: Attempt,
    recorded: Option<(JobLease, i64)>,
    ended: Ended,
) {
    let Ended {
        bound,
        delivered,
        attempt_started_at,
    } = ended;
    match (attempt, recorded) {
        (Attempt::Landed(count), Some(_)) => {
            if let Some(ask) = bound {
                if !delivered || rebound(job, &ask) {
                    follow_up(job, &ask, count, Landing::Landed).await;
                }
            }
        }
        (Attempt::Landed(count), None) if !delivered => {
            follow_up(job, &own_ask(job), count, Landing::Landed).await;
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
            give_back(job, claim, &lease, attempts).await;
        }
        (Attempt::Failed, Some((lease, attempts))) => {
            // The turn heard of the failure itself; only a newer ask the
            // row was rebound to is still owed. It keeps the row while the
            // read has attempts left, and is told the read gave up once it
            // has none.
            let newer = finish(job, &lease).await.filter(|ask| rebound(job, ask));
            if let Some(ask) = newer {
                if attempts >= MAX_BACKFILL_ATTEMPTS {
                    answer_given_up(job, claim, &ask).await;
                } else {
                    owe(job, ask, attempts).await;
                }
            }
        }
        (Attempt::Landed(_) | Attempt::Failed, None) => {}
    }
}

/// A failed attempt no turn heard of: the row stays owed for the resume
/// sweep's next attempt, its lease given back so that every ask meanwhile
/// finds it idle and reads for itself instead of joining a read nothing
/// runs ([`take_over_idle`]). The sweep waits a lease's length before that
/// attempt, as it did while a failed attempt kept its lease, so a failing
/// provider is not spent through every attempt in minutes. Its last attempt
/// gives up instead ([`give_up`]).
async fn give_back(
    job: &ActivityBackfillJob,
    claim: &InFlightClaim,
    lease: &JobLease,
    attempts: i64,
) {
    if attempts >= MAX_BACKFILL_ATTEMPTS {
        give_up(job, claim, lease, attempts).await;
        return;
    }
    match job
        .resources
        .repos()
        .activity_backfill_jobs
        .release_backfill_job_lease(lease, now_ms().saturating_add(lease_ms()))
        .await
    {
        Ok(released) => info!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %lease,
            attempts,
            released,
            "Live read: failed; its row stays owed for a retry, idle until the sweep or the next ask takes it"
        ),
        Err(e) => warn!(
            user_id = %job.user_id,
            provider = %job.provider_name,
            row_id = %lease,
            error = %e,
            "Live read: the failed read's lease could not be given back; it lapses on its own"
        ),
    }
}

/// The read's last attempt failed and no turn heard of it: drop its row, so
/// the athlete's next ask starts a fresh read, and give the ask it was bound
/// to the follow-up it was promised ([`answer_given_up`]). A row bound to no
/// conversation (a stale head's top-up) owes nobody anything.
async fn give_up(
    job: &ActivityBackfillJob,
    claim: &InFlightClaim,
    lease: &JobLease,
    attempts: i64,
) {
    warn!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        row_id = %lease,
        attempts,
        "Live read: failed on its last attempt; dropping the job and telling the ask it was bound to"
    );
    if let Some(ask) = finish(job, lease).await {
        answer_given_up(job, claim, &ask).await;
    }
}

/// Follow `ask` up for a read that gave up: the run reads
/// [`InFlight::GaveUp`] first, so the follow-up's re-ask joins it and is
/// answered from what is stored, the provider unavailable, without reading
/// it again.
async fn answer_given_up(job: &ActivityBackfillJob, claim: &InFlightClaim, ask: &LiveReadAsk) {
    mark_in_flight(claim, InFlight::GaveUp);
    follow_up(job, ask, 0, Landing::GaveUp).await;
}

/// Follow up the ask a reaped read was bound to (`job` rebuilt from its
/// row): its attempts are spent and its last runner died, so it gave up as
/// [`give_up`] does, with no row left to finish. `claim` holds the pair's
/// in-flight key.
pub(super) async fn answer_reaped(job: &ActivityBackfillJob, claim: &InFlightClaim) {
    warn!(
        user_id = %job.user_id,
        provider = %job.provider_name,
        "Live read: reaped past its attempts after its last runner died; telling the ask it was bound to"
    );
    answer_given_up(job, claim, &own_ask(job)).await;
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
    time::timeout(cap, read).await.map_or_else(
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

/// Whether the read a follow-up answers for landed, or gave up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Landing {
    /// The read landed its rows.
    Landed,
    /// Every attempt failed: the provider stayed unavailable.
    GaveUp,
}

/// Answer the conversation `ask` names, now that the read landed or gave up:
/// a re-ask of the athlete's question, or a note that its data landed when
/// they have asked something newer since — the notifier tells which.
async fn follow_up(
    job: &ActivityBackfillJob,
    ask: &LiveReadAsk,
    activity_count: usize,
    landing: Landing,
) {
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
        gave_up = landing == Landing::GaveUp,
        "Live read ended after its turn; answering the conversation that asked"
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
            provider_unavailable: landing == Landing::GaveUp,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(
        after: Option<i64>,
        before: Option<i64>,
        limit: Option<usize>,
    ) -> ActivityQueryParams {
        ActivityQueryParams {
            limit,
            offset: None,
            before,
            after,
        }
    }

    #[test]
    fn a_read_covers_a_later_ask_of_its_window() {
        let read = window(Some(1_000), None, Some(50));
        assert!(
            covers(&read, 12, &window(Some(1_000), None, Some(50))),
            "bug: a read covers the same ask again"
        );
        assert!(
            covers(&read, 12, &window(Some(1_060), None, Some(200))),
            "a read that ran out before its limit saw the whole window, whatever the ask's limit"
        );
        assert!(
            covers(&read, 12, &window(Some(1_060), Some(5_000), Some(10))),
            "bug: a read covers a closed window inside its own"
        );
    }

    #[test]
    fn a_read_does_not_cover_what_it_did_not_reach() {
        let read = window(Some(1_000), None, Some(50));
        assert!(
            !covers(&read, 12, &window(Some(900), None, Some(50))),
            "older than the read's floor"
        );
        assert!(
            !covers(&read, 12, &window(None, None, Some(50))),
            "an unbounded ask reaches below any floor"
        );
        assert!(
            !covers(&read, 50, &window(Some(1_000), None, Some(200))),
            "a read cut at its limit did not reach the bottom of a deeper ask"
        );
        assert!(
            covers(&read, 50, &window(Some(1_000), None, Some(50))),
            "bug: a read cut at its limit still covers an ask no deeper than it"
        );
        let paged = ActivityQueryParams {
            offset: Some(10),
            ..window(Some(1_000), None, Some(50))
        };
        assert!(
            !covers(&read, 50, &paged),
            "a page below the read's limit was not read"
        );
        let closed = window(Some(1_000), Some(4_000), Some(50));
        assert!(
            !covers(&closed, 12, &window(Some(1_000), None, Some(50))),
            "newer than a closed read's top"
        );
        let skipped = ActivityQueryParams {
            offset: Some(5),
            ..read
        };
        assert!(
            !covers(&skipped, 12, &window(Some(1_000), None, Some(50))),
            "a read that skipped its newest rows"
        );
    }
}
