// ABOUTME: Returns the pushes an armed persona floor withheld, as digests on the contract's cadence
// ABOUTME: A daily tick sends daily, weekly and per-athlete digests; a landed session sends per-session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Persona notification digests
//!
//! The armed persona notification policy persists a gated push instead of
//! delivering it (see `pierre_notifications::NotificationService::dispatch_with_tier`),
//! and the persona contracts promise those held notifications come back as a
//! digest on the persona's cadence. This module keeps that promise for every
//! [`DigestCadence`]:
//!
//! - `daily` — on the daily [`tick`], at most one digest a day.
//! - `weekly` — on the daily [`tick`], once seven days have passed since the
//!   recipient's previous digest.
//! - `per_athlete` — on the daily [`tick`], at most once a day: one digest
//!   per athlete the held notifications concern (the athlete a notification
//!   names under [`SubjectAthlete`]), plus one daily digest for those that
//!   concern the recipient themselves.
//! - `per_session` — when the athlete's next training session lands in the
//!   activity cache ([`session_landed`], called by the cache write-through
//!   every provider fetch goes through). The tick never sends it.
//!
//! A recipient with no digest yet is due at once; after that a calendar
//! cadence waits out its period, measured from the newest digest of any
//! cadence.
//!
//! ## What a digest returns
//!
//! A held notification is a persisted row carrying the `persona_gated`
//! marker. Before a digest is sent it claims the held rows it returns in
//! `persona_digest_returns` (see
//! [`PersonaDigestReturnRepository`](pierre_database::repositories::PersonaDigestReturnRepository)),
//! keyed by the held row, so a held row is returned by exactly one digest:
//! the backlog is every held row no digest has claimed. A restart, an extra
//! tick, a second instance or two sessions landing at once therefore re-send
//! nothing, a row held while a digest was being sent waits for the next one
//! rather than being skipped, and a digest that did not go out releases its
//! claim so its rows wait for the next one. The claim is the digest's own
//! record, not a field of the digest notification: an athlete deleting the
//! digest from their feed does not make its rows due again. The digest
//! carries only the claim's id, under [`DIGEST_BATCH_PARAM`], which keeps its
//! push payload the same size however many rows it returns.
//!
//! Every digest is a localized System notification dispatched at
//! [`PushTier::P0`] — P0 so the digest itself can never be persona-gated.
//!
//! The tick mirrors the tick/run-loop shape of
//! `pierre_routes_groups::group_digest_scheduler`: a Tokio task spawned at
//! server bootstrap ticks every [`DEFAULT_TICK_INTERVAL`], and the testable
//! [`tick`] does one full sweep.

use pierre_core::transport::TransportPolicy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::periodic::spawn_periodic;
use chrono::{Duration, Utc};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantId, User};
use pierre_database::RepositoryRegistry;
use pierre_notifications::events::{data_transport_policy, event_data, stamp_data};
use pierre_notifications::models::{Notification, NotificationCategory};
use pierre_notifications::{
    to_app_error, DigestCadence, DispatchOutcome, DispatchRequest, NotificationEvent,
    NotificationService, PushTier, SubjectAthlete, TenantId as CommTenantId,
    PERSONA_GATED_DATA_KEY,
};
use serde_json::{json, Map, Value};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::notification_text::NotificationTextRenderer;

/// How often the digest tick fires. One day — the shortest calendar cadence
/// the contracts name; longer cadences wait out their period across ticks.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_hours(24);

/// `notification_type` of the weekly digest.
pub const PERSONA_DIGEST_TYPE: &str = NotificationEvent::PersonaDigest.wire();

/// Digest parameter naming the claim in `persona_digest_returns` that holds
/// the ids of the held rows the digest returns.
pub const DIGEST_BATCH_PARAM: &str = "digest_batch";

/// Upper bound on notifications examined per user per sweep. Bounds the scan
/// the way pagination clamps a list endpoint; a user holding more rows than
/// this still gets a digest, of the newest rows.
const DIGEST_SCAN_LIMIT: u32 = 500;

/// Slack under a calendar cadence's period before the next digest is due.
///
/// The ledger starts each tick one period after the previous tick
/// *finished*, while the previous digest was stamped partway through it, so
/// consecutive digests sit a little under one period apart on the database
/// clock; instance clocks differ by a little too. An hour absorbs both and is
/// far below the shortest period.
const DUE_TOLERANCE: Duration = Duration::hours(1);

/// Outcome of a single scheduler tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PersonaDigestTickOutcome {
    /// Tenants examined this tick.
    pub tenants_scanned: usize,
    /// Active users examined across all tenants.
    pub users_scanned: usize,
    /// Users whose armed policy carries a cadence the tick delivers
    /// (`daily`, `weekly`, `per_athlete`).
    pub users_eligible: usize,
    /// Digest notifications dispatched (a `per_athlete` user can get several).
    pub digests_sent: usize,
    /// Per-user errors. Logged; the sweep continues.
    pub errors: usize,
}

/// Run a single digest sweep across all tenants.
///
/// Exposed (rather than only the loop) so integration tests can drive one
/// sweep without `tokio::sleep`. Production callers wrap this in
/// [`start_persona_digest_scheduler`]. Each user's policy is read through
/// `service`'s own persona gate — the one its dispatches were withheld by.
///
/// # Errors
///
/// Returns the database error only if the initial tenant enumeration fails.
/// Per-tenant and per-user errors are counted in
/// [`PersonaDigestTickOutcome::errors`] but do not abort the sweep.
pub async fn tick(
    repos: &Arc<RepositoryRegistry>,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
) -> AppResult<PersonaDigestTickOutcome> {
    let tenants = repos.tenants.get_all().await?;

    let mut outcome = PersonaDigestTickOutcome {
        tenants_scanned: tenants.len(),
        ..PersonaDigestTickOutcome::default()
    };

    for tenant in tenants {
        process_tenant(repos, service, strings, tenant.id, &mut outcome).await;
    }

    if outcome.users_eligible > 0 {
        info!(
            tenants_scanned = outcome.tenants_scanned,
            users_scanned = outcome.users_scanned,
            users_eligible = outcome.users_eligible,
            digests_sent = outcome.digests_sent,
            errors = outcome.errors,
            "persona notification digest sweep complete"
        );
    } else {
        debug!(
            tenants_scanned = outcome.tenants_scanned,
            users_scanned = outcome.users_scanned,
            "persona notification digest sweep: no armed calendar-cadence user"
        );
    }

    Ok(outcome)
}

/// Deliver the `per_session` digest: one of `user_id`'s training sessions
/// just landed.
///
/// Sends one digest of everything withheld since the athlete's previous
/// digest when their policy is armed with the `per_session` cadence and
/// anything was withheld; otherwise sends nothing. Returns how many digests
/// were dispatched.
///
/// # Errors
///
/// Returns the database or dispatch error that stopped the digest.
pub async fn session_landed(
    repos: &RepositoryRegistry,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<usize> {
    let tenant = CommTenantId(tenant_id.as_uuid());
    let Some(push_policy) = service.push_policy(user_id, tenant).await else {
        return Ok(0);
    };
    if !push_policy.armed || push_policy.digest != Some(DigestCadence::PerSession) {
        return Ok(0);
    }
    let Some(user) = repos.users.get(user_id, tenant_id).await? else {
        return Ok(0);
    };
    let held = held_backlog(repos, service, user_id, tenant_id).await?;
    if held.is_empty() {
        return Ok(0);
    }
    let sent = Digests::new(repos, service, strings, &user, tenant_id)
        .deliver(DigestCadence::PerSession, held)
        .await?;
    debug!(user_id = %user_id, sent, "per-session persona digest delivered");
    Ok(sent)
}

/// Sweep one tenant's active users, digesting each armed user whose calendar
/// cadence is due.
async fn process_tenant(
    repos: &Arc<RepositoryRegistry>,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
    tenant_id: TenantId,
    outcome: &mut PersonaDigestTickOutcome,
) {
    let users = match repos.users.get_by_status("active", Some(tenant_id)).await {
        Ok(users) => users,
        Err(e) => {
            error!(tenant_id = %tenant_id, error = %e, "persona digest: user listing failed");
            outcome.errors += 1;
            return;
        }
    };
    let tenant = CommTenantId(tenant_id.as_uuid());
    for user in users {
        outcome.users_scanned += 1;
        let Some(push_policy) = service.push_policy(user.id, tenant).await else {
            continue;
        };
        let Some(cadence) = push_policy.digest.filter(|_| push_policy.armed) else {
            continue;
        };
        // `per_session` is the athlete's training's to decide, never the clock's.
        let Some(period) = calendar_period(cadence) else {
            continue;
        };
        outcome.users_eligible += 1;
        match digest_when_due(repos, service, strings, &user, tenant_id, cadence, period).await {
            Ok(sent) => outcome.digests_sent += sent,
            Err(e) => {
                warn!(
                    user_id = %user.id,
                    tenant_id = %tenant_id,
                    cadence = %cadence,
                    error = %e,
                    "persona digest: dispatch failed (best-effort)"
                );
                outcome.errors += 1;
            }
        }
    }
}

/// How long a calendar cadence waits between two digests, or `None` for
/// `per_session`, which the tick never sends.
const fn calendar_period(cadence: DigestCadence) -> Option<Duration> {
    match cadence {
        DigestCadence::Daily | DigestCadence::PerAthlete => Some(Duration::days(1)),
        DigestCadence::Weekly => Some(Duration::days(7)),
        DigestCadence::PerSession => None,
    }
}

/// Send `user`'s backlog on `cadence` when anything is held and their newest
/// digest is at least `period` old. Returns how many digests were dispatched.
async fn digest_when_due(
    repos: &RepositoryRegistry,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
    user: &User,
    tenant_id: TenantId,
    cadence: DigestCadence,
    period: Duration,
) -> AppResult<usize> {
    let held = held_backlog(repos, service, user.id, tenant_id).await?;
    if held.is_empty() {
        return Ok(0);
    }
    let previous = repos
        .persona_digest_returns
        .last_persona_digest_at(user.id, tenant_id)
        .await?;
    if previous.is_some_and(|previous| Utc::now() - previous < period - DUE_TOLERANCE) {
        return Ok(0);
    }
    Digests::new(repos, service, strings, user, tenant_id)
        .deliver(cadence, held)
        .await
}

/// The recipient's held rows no digest has returned yet, newest first.
///
/// Reads the recipient's newest notifications, keeps those carrying the
/// persona-gated marker, and drops those a digest already claimed.
async fn held_backlog(
    repos: &RepositoryRegistry,
    service: &NotificationService,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<Vec<Notification>> {
    let (rows, _, _) = service
        .list_notifications(
            user_id,
            CommTenantId(tenant_id.as_uuid()),
            DIGEST_SCAN_LIMIT,
            0,
            None,
            false,
        )
        .await
        .map_err(to_app_error)?;
    let held: Vec<Notification> = rows
        .into_iter()
        .filter(|n| is_persona_gated(n.data.as_ref()))
        .collect();
    let Some(oldest) = held.iter().map(|n| n.created_at).min() else {
        return Ok(held);
    };
    let returned = repos
        .persona_digest_returns
        .persona_digest_items_returned_since(user_id, tenant_id, oldest)
        .await?;
    Ok(held
        .into_iter()
        .filter(|n| !returned.contains(&n.id))
        .collect())
}

/// Whether a persisted notification carries the persona-gated marker.
fn is_persona_gated(data: Option<&Value>) -> bool {
    data.and_then(|d| d.get(PERSONA_GATED_DATA_KEY))
        .and_then(Value::as_bool)
        == Some(true)
}

/// Sends one recipient's digests, rendered in their own language.
struct Digests<'a> {
    repos: &'a RepositoryRegistry,
    service: &'a NotificationService,
    renderer: NotificationTextRenderer<'a>,
    user: &'a User,
    tenant_id: TenantId,
}

impl<'a> Digests<'a> {
    fn new(
        repos: &'a RepositoryRegistry,
        service: &'a NotificationService,
        strings: &'a MessagingStringsRegistry,
        user: &'a User,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            repos,
            service,
            renderer: NotificationTextRenderer::new(strings, &user.locale),
            user,
            tenant_id,
        }
    }

    /// Return `held` as `cadence` promises: one digest, or for `per_athlete`
    /// one per athlete plus one for the recipient's own. Returns how many
    /// digests were dispatched.
    async fn deliver(&self, cadence: DigestCadence, held: Vec<Notification>) -> AppResult<usize> {
        let event = match cadence {
            DigestCadence::Daily => NotificationEvent::PersonaDailyDigest,
            DigestCadence::Weekly => NotificationEvent::PersonaDigest,
            DigestCadence::PerSession => NotificationEvent::PersonaSessionDigest,
            DigestCadence::PerAthlete => return self.deliver_per_athlete(held).await,
        };
        Ok(usize::from(self.send(event, &held, None).await?))
    }

    /// One digest per athlete the held rows concern, and one daily digest for
    /// the rows that concern the recipient themselves.
    async fn deliver_per_athlete(&self, held: Vec<Notification>) -> AppResult<usize> {
        let (own, by_athlete) = split_by_subject(held, self.user.id);
        let mut sent = 0;
        if !own.is_empty() {
            sent += usize::from(
                self.send(NotificationEvent::PersonaDailyDigest, &own, None)
                    .await?,
            );
        }
        for (subject, rows) in &by_athlete {
            sent += usize::from(
                self.send(NotificationEvent::PersonaAthleteDigest, rows, Some(subject))
                    .await?,
            );
        }
        Ok(sent)
    }

    /// Claim `rows` and dispatch one digest returning those this digest
    /// claimed. Returns whether a digest went out.
    ///
    /// A row another digest claimed first (a concurrent landing or sweep) is
    /// left to it; when every row was, nothing is sent. A digest the pipeline
    /// suppressed (quiet hours, a disabled category, the daily cap) stores
    /// nothing, and one that failed may not have, so both release their claim
    /// and the rows stay held for the next digest.
    async fn send(
        &self,
        event: NotificationEvent,
        rows: &[Notification],
        subject: Option<&SubjectAthlete>,
    ) -> AppResult<bool> {
        let batch_id = Uuid::new_v4();
        let ids: Vec<Uuid> = rows.iter().map(|n| n.id).collect();
        let claimed = self
            .repos
            .persona_digest_returns
            .claim_persona_digest_items(self.user.id, self.tenant_id, batch_id, &ids, Utc::now())
            .await?;
        if claimed.is_empty() {
            return Ok(false);
        }
        let mut params = Map::new();
        params.insert("item_count".to_owned(), json!(claimed.len()));
        params.insert(DIGEST_BATCH_PARAM.to_owned(), json!(batch_id.to_string()));
        if let Some(subject) = subject {
            params.insert("athlete_name".to_owned(), json!(subject.name));
            params.insert("athlete_id".to_owned(), json!(subject.id.to_string()));
        }
        // The digest is dispatched directly rather than through
        // `dispatch_event` because the sweep already holds the user row it
        // would re-read; the text comes from the same renderer either way, so
        // both paths say the same thing, and the stored parameters let the
        // feed re-render it after a language change.
        let request = DispatchRequest {
            user_id: self.user.id,
            tenant_id: CommTenantId(self.tenant_id.as_uuid()),
            category: NotificationCategory::System,
            notification_type: event.wire().to_owned(),
            title: self.renderer.title(event, &params),
            body: self.renderer.body(event, &params),
            // As strict as the notifications it rolls up (carnet#769).
            data: stamp_data(
                Some(event_data(json!({}), Value::Object(params))),
                TransportPolicy::strictest_of(
                    rows.iter().map(|n| data_transport_policy(n.data.as_ref())),
                ),
            ),
            image_url: None,
            actions: None,
            bypass_frequency_cap: false,
        };
        // P0: the digest is the delivery the persona floor promised in
        // exchange for the pushes it withheld, so it must never gate itself.
        let outcome = self
            .service
            .dispatch_with_tier(&request, PushTier::P0)
            .await
            .map_err(to_app_error);
        let accepted = match outcome {
            Ok(DispatchOutcome::Suppressed(_)) => false,
            Ok(_) => true,
            Err(e) => {
                self.release(batch_id).await;
                return Err(e);
            }
        };
        if !accepted {
            self.release(batch_id).await;
        }
        debug!(
            user_id = %self.user.id,
            notification_type = event.wire(),
            item_count = claimed.len(),
            accepted,
            "persona digest dispatched"
        );
        Ok(accepted)
    }

    /// Release the claim of a digest that did not go out. A failed release is
    /// logged: its rows then stay claimed by a digest nobody received, and
    /// are visible in the in-app list only.
    async fn release(&self, batch_id: Uuid) {
        if let Err(e) = self
            .repos
            .persona_digest_returns
            .release_persona_digest_batch(self.user.id, self.tenant_id, batch_id)
            .await
        {
            warn!(
                user_id = %self.user.id,
                batch_id = %batch_id,
                error = %e,
                "persona digest: releasing an undelivered digest's claim failed"
            );
        }
    }
}

/// Split held rows into those about the recipient and those about each other
/// athlete they name, newest athlete first. An athlete is named as the newest
/// of their rows names them.
fn split_by_subject(
    held: Vec<Notification>,
    recipient: Uuid,
) -> (Vec<Notification>, Vec<(SubjectAthlete, Vec<Notification>)>) {
    let mut own = Vec::new();
    let mut by_athlete: Vec<(SubjectAthlete, Vec<Notification>)> = Vec::new();
    let mut slot: HashMap<Uuid, usize> = HashMap::new();
    for row in held {
        let Some(subject) =
            SubjectAthlete::from_data(row.data.as_ref()).filter(|s| s.id != recipient)
        else {
            own.push(row);
            continue;
        };
        let index = *slot.entry(subject.id).or_insert_with(|| {
            by_athlete.push((subject, Vec::new()));
            by_athlete.len() - 1
        });
        by_athlete[index].1.push(row);
    }
    (own, by_athlete)
}

/// Spawn the persona-digest scheduler as a background tokio task.
///
/// Called once at server bootstrap beside the group digest spawn, with the
/// notification service whose persona gate withholds the pushes. The
/// [`AbortHandle`](tokio::task::AbortHandle) is discarded because the scheduler
/// is best-effort and the worker ledger carries the schedule across restarts;
/// "already returned" is derived from the persisted digests, so a retry or a
/// second instance re-sends nothing.
pub fn start_persona_digest_scheduler(
    repos: Arc<RepositoryRegistry>,
    service: Arc<NotificationService>,
    strings: Arc<MessagingStringsRegistry>,
) {
    let ledger = Arc::clone(&repos.worker_runs);
    spawn_periodic(
        "persona notification digest scheduler",
        DEFAULT_TICK_INTERVAL,
        ledger,
        move || {
            let repos = Arc::clone(&repos);
            let service = Arc::clone(&service);
            let strings = Arc::clone(&strings);
            async move {
                tick(&repos, &service, &strings).await?;
                Ok(())
            }
        },
    );
}
