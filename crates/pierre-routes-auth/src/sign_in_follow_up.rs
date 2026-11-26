// ABOUTME: The check queued ten minutes after a provider refused a sign-in: is the athlete connected by now?
// ABOUTME: Holds the refusal window's cache keys, the last-refusal record, the queue trait and the two ops lines
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The follow-up of a refused sign-in (carnet#1837).
//!
//! A provider refusing an athlete's credentials is not, by itself, an athlete
//! stuck: they retype the password, switch to OAuth, and connect. What ops
//! needs to see is an athlete still not connected some time after the last
//! refusal. So every refusal records itself as the athlete's last one
//! ([`LastRefusal`], under the user-facing provider) and queues a
//! [`FollowUp`] for [`FOLLOW_UP_DELAY`] later. Only the follow-up of the
//! latest refusal acts ([`check`]): it reads whether any backend serving the
//! provider now holds a live connection, and logs either
//! [`STILL_NOT_CONNECTED`] at WARN, which the ops Slack alert matches, or
//! [`RECOVERED`] at INFO.
//!
//! The live environment scales to zero, so in production the follow-up is a
//! Cloud Tasks task the composition root queues; locally the process holds it
//! as a timer ([`InProcessFollowUps`]).

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dravr_tronc::dates::Timestamp;
use pierre_cache::{Cache, CacheKey, CacheResource};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{
    first_readable_backend, serving_backends, user_facing_name,
};
use serde::{Deserialize, Serialize};
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

/// How long refusals count together: the ordinal on each refusal line is the
/// refusal's rank in a window this long, opened by the first.
pub(crate) const REFUSAL_WINDOW: Duration = Duration::from_mins(30);

/// How long after a refusal its follow-up checks the connection.
pub(crate) const FOLLOW_UP_DELAY: Duration = Duration::from_mins(10);

/// The path a queued follow-up is delivered to.
pub const FOLLOW_UP_PATH: &str = "/internal/sign-in/follow-up";

/// The tracing target both follow-up lines are pinned to, so the alert's
/// match holds wherever the code lives.
pub(crate) const FOLLOW_UP_TARGET: &str = "pierre_routes_auth::sign_in_follow_up";

/// The WARN a follow-up writes when the athlete is still not connected.
/// `infra/environments/dev/provider_sign_in_monitoring.tf` matches it.
pub(crate) const STILL_NOT_CONNECTED: &str = "Provider sign-in still not connected after refusals";

/// The INFO a follow-up writes when the athlete connected after all.
pub(crate) const RECOVERED: &str = "Provider sign-in recovered after refusals";

/// A check queued for one refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowUp {
    /// The tenant the refused sign-in ran in.
    pub tenant_id: Uuid,
    /// The athlete.
    pub user_id: Uuid,
    /// The backend the refused sign-in targeted (`sciotte_garmin`).
    pub backend: String,
    /// When the refusal happened, epoch milliseconds: the follow-up acts
    /// only while this is still the athlete's last refusal.
    pub refused_at_ms: i64,
}

impl FollowUp {
    /// The task id of this follow-up, one per refusal.
    ///
    /// A later refusal queues a task of its own. Cloud Tasks ids take
    /// letters, digits, `-` and `_`.
    #[must_use]
    pub fn task_id(&self) -> String {
        format!("sign-in-follow-up-{}-{}", self.user_id, self.refused_at_ms)
    }

    /// When the follow-up is due: [`FOLLOW_UP_DELAY`] after the refusal.
    /// `None` for milliseconds no instant has.
    #[must_use]
    pub fn due_at(&self) -> Option<Timestamp> {
        let delay_ms = i64::try_from(FOLLOW_UP_DELAY.as_millis()).unwrap_or(i64::MAX);
        Timestamp::from_epoch_millis(self.refused_at_ms.saturating_add(delay_ms)).ok()
    }
}

/// Queues follow-ups for their time.
#[async_trait]
pub trait SignInFollowUps: Send + Sync + fmt::Debug {
    /// Queue `follow_up` for [`FollowUp::due_at`].
    ///
    /// # Errors
    ///
    /// Returns an error when the follow-up could not be queued. The caller
    /// logs it and answers the sign-in as it would have: a lost follow-up
    /// costs one ops line, never the athlete's response.
    async fn queue(&self, follow_up: FollowUp) -> AppResult<()>;
}

/// The athlete's last refusal for one provider, as the ops log named it:
/// the reason and the account masked, as they are on the refusal line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LastRefusal {
    /// When it happened, epoch milliseconds.
    pub(crate) refused_at_ms: i64,
    /// The provider's reason, any address in it masked.
    pub(crate) reason: String,
    /// How the athlete signed in (`email`, `google`, `apple`, `continuation`).
    pub(crate) method_used: String,
    /// The account typed at the provider, masked; empty for a continuation.
    pub(crate) account: String,
    /// Its rank among the window's refusals, 0 when the count failed.
    pub(crate) refusals: u64,
}

/// Where the refusal window of `user_id`'s sign-ins to `provider` counts.
///
/// Keyed by the user-facing provider, so both of Garmin's backends count
/// into one window.
#[must_use]
pub(crate) fn refusals_key(tenant_id: Uuid, user_id: Uuid, provider: &str) -> CacheKey {
    CacheKey::new(
        TenantId::from_uuid(tenant_id),
        user_id,
        user_facing_name(provider).to_owned(),
        CacheResource::SignInRefusals,
    )
}

/// Where `user_id`'s last refusal for `provider` is kept, keyed by the
/// user-facing provider as [`refusals_key`] is.
#[must_use]
pub(crate) fn last_refusal_key(tenant_id: Uuid, user_id: Uuid, provider: &str) -> CacheKey {
    CacheKey::new(
        TenantId::from_uuid(tenant_id),
        user_id,
        user_facing_name(provider).to_owned(),
        CacheResource::SignInLastRefusal,
    )
}

/// What a follow-up found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowUpVerdict {
    /// A later refusal replaced the one it was queued for, or the record is
    /// gone (it lapsed, or an earlier delivery of this follow-up acted).
    Superseded,
    /// No backend serving the provider holds a live connection.
    StillNotConnected,
    /// The athlete connected; the backend that serves them now.
    Recovered(String),
}

impl FollowUpVerdict {
    /// The verdict as the delivery's body names it.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::StillNotConnected => "still_not_connected",
            Self::Recovered(_) => "recovered",
        }
    }
}

/// Run `follow_up` at `now`: say whether the athlete connected.
///
/// It logs what it found, then forgets the window while it is still this
/// refusal's, so the next refusal starts a new one.
///
/// Connected means a backend serving the provider ([`serving_backends`],
/// OAuth or sign-in alike) holds a live token row in the follow-up's tenant
/// ([`first_readable_backend`]): a refused re-sign-in leaves the athlete's dead session
/// row in place, and that row is not a connection.
///
/// # Errors
///
/// When the token rows or connections cannot be read, so the delivery fails
/// and Cloud Tasks retries it. A cache failure reading the record reads as
/// [`FollowUpVerdict::Superseded`]: retrying would not bring it back.
pub async fn check(
    cache: &Cache,
    repos: &RepositoryRegistry,
    follow_up: &FollowUp,
    now: Timestamp,
) -> AppResult<FollowUpVerdict> {
    let FollowUp {
        tenant_id,
        user_id,
        ref backend,
        refused_at_ms,
    } = *follow_up;
    let last_key = last_refusal_key(tenant_id, user_id, backend);
    let last = match cache.get::<LastRefusal>(&last_key).await {
        Ok(last) => last,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Could not read the last sign-in refusal; dropping its follow-up");
            None
        }
    };
    let Some(last) = last.filter(|last| last.refused_at_ms == refused_at_ms) else {
        return Ok(FollowUpVerdict::Superseded);
    };

    let provider = user_facing_name(backend);
    let routes = serving_backends(provider);
    let tenant = TenantId::from_uuid(tenant_id);
    let connected = first_readable_backend(repos, user_id, tenant, &routes, now).await?;
    let minutes = now.epoch_millis().saturating_sub(refused_at_ms).max(0) / 60_000;
    log_outcome(follow_up, &last, &routes, minutes, connected.as_deref());
    let verdict = connected.map_or(
        FollowUpVerdict::StillNotConnected,
        FollowUpVerdict::Recovered,
    );

    // Forget the window only while it is still this refusal's: a refusal that
    // landed during the check wrote its own record, whose follow-up must act.
    let still_ours = matches!(
        cache.get::<LastRefusal>(&last_key).await,
        Ok(Some(current)) if current.refused_at_ms == refused_at_ms
    );
    if still_ours {
        for key in [last_key, refusals_key(tenant_id, user_id, backend)] {
            if let Err(e) = cache.invalidate(&key).await {
                warn!(user_id = %user_id, error = %e, "Could not clear a sign-in refusal entry; it lapses on its own");
            }
        }
    }
    Ok(verdict)
}

/// Log what a follow-up found.
///
/// [`RECOVERED`] at INFO naming the route that connected, or
/// [`STILL_NOT_CONNECTED`] at WARN with what the ops alert posts. Every
/// value on either line is an id, a count or already masked.
fn log_outcome(
    follow_up: &FollowUp,
    last: &LastRefusal,
    routes: &[String],
    minutes: i64,
    connected: Option<&str>,
) {
    let provider = user_facing_name(&follow_up.backend);
    let refusals = last.refusals.max(1);
    let Some(route) = connected else {
        warn!(
            target: FOLLOW_UP_TARGET,
            user_id = %follow_up.user_id,
            tenant_id = %follow_up.tenant_id,
            provider = %provider,
            backend = %follow_up.backend,
            refusals,
            minutes,
            reason = %last.reason,
            method_used = %last.method_used,
            account = %last.account,
            routes_checked = %routes.join(","),
            "{STILL_NOT_CONNECTED}"
        );
        return;
    };
    info!(
        target: FOLLOW_UP_TARGET,
        user_id = %follow_up.user_id,
        tenant_id = %follow_up.tenant_id,
        provider = %provider,
        backend = %follow_up.backend,
        refusals,
        minutes,
        route = %route,
        "{RECOVERED}"
    );
}

/// Local [`SignInFollowUps`]: holds each follow-up as a timer in the running
/// process, for wherever turns run in-process (no queue to wake it).
#[derive(Clone)]
pub struct InProcessFollowUps {
    cache: Arc<Cache>,
    repos: Arc<RepositoryRegistry>,
}

impl InProcessFollowUps {
    /// Follow-ups checked against this cache and these repositories.
    #[must_use]
    pub const fn new(cache: Arc<Cache>, repos: Arc<RepositoryRegistry>) -> Self {
        Self { cache, repos }
    }
}

impl fmt::Debug for InProcessFollowUps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InProcessFollowUps").finish_non_exhaustive()
    }
}

#[async_trait]
impl SignInFollowUps for InProcessFollowUps {
    async fn queue(&self, follow_up: FollowUp) -> AppResult<()> {
        let wait = follow_up.due_at().map_or(Duration::ZERO, |due| {
            let left = due
                .epoch_millis()
                .saturating_sub(Timestamp::now().epoch_millis());
            Duration::from_millis(u64::try_from(left).unwrap_or(0))
        });
        let cache = Arc::clone(&self.cache);
        let repos = Arc::clone(&self.repos);
        let timer = tokio::spawn(async move {
            sleep(wait).await;
            if let Err(e) = check(&cache, &repos, &follow_up, Timestamp::now()).await {
                warn!(task = %follow_up.task_id(), error = %e, "A sign-in follow-up could not read the connection");
            }
        });
        // The timer's own end is watched, so a panic in it is logged rather
        // than lost with a dropped handle.
        tokio::spawn(async move {
            if let Err(e) = timer.await {
                warn!(error = %e, "A sign-in follow-up timer ended without running its check");
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use pierre_test_support::log_capture::{capture_events_while, CapturedEvent};
    use tracing::Level;

    use super::*;

    #[test]
    fn both_backends_of_a_provider_share_one_window_and_one_record() {
        let (tenant, user) = (Uuid::from_u128(1), Uuid::from_u128(2));
        assert_eq!(
            refusals_key(tenant, user, "sciotte_garmin").to_string(),
            refusals_key(tenant, user, "garmin").to_string(),
            "bug: a refusal on the mirror and one on OAuth count apart"
        );
        assert_eq!(
            last_refusal_key(tenant, user, "sciotte").to_string(),
            last_refusal_key(tenant, user, "strava").to_string(),
        );
        assert_ne!(
            refusals_key(tenant, user, "garmin").to_string(),
            refusals_key(tenant, Uuid::from_u128(3), "garmin").to_string(),
            "bug: two athletes count into one window"
        );
        assert_ne!(
            refusals_key(tenant, user, "garmin").to_string(),
            last_refusal_key(tenant, user, "garmin").to_string(),
            "bug: the counter and the record share a key"
        );
    }

    #[test]
    fn each_refusal_has_its_own_task_due_ten_minutes_later() {
        let first = FollowUp {
            tenant_id: Uuid::nil(),
            user_id: Uuid::from_u128(9),
            backend: "sciotte_garmin".to_owned(),
            refused_at_ms: 1_764_000_000_000,
        };
        let second = FollowUp {
            refused_at_ms: first.refused_at_ms + 1,
            ..first.clone()
        };
        assert_ne!(
            first.task_id(),
            second.task_id(),
            "bug: a later refusal's task is refused as a duplicate"
        );
        assert!(
            first
                .task_id()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "bug: a task id holds a character Cloud Tasks refuses"
        );
        assert_eq!(
            first.due_at().map(Timestamp::epoch_millis),
            Some(first.refused_at_ms + 600_000)
        );
    }

    #[test]
    fn a_follow_up_round_trips_through_its_body() {
        let follow_up = FollowUp {
            tenant_id: Uuid::from_u128(4),
            user_id: Uuid::from_u128(5),
            backend: "sciotte".to_owned(),
            refused_at_ms: 42,
        };
        let body = serde_json::to_value(&follow_up).unwrap(); // Safe: a plain struct serializes
        let back: FollowUp = serde_json::from_value(body).unwrap(); // Safe: it was just written
        assert_eq!(back, follow_up);
    }

    fn still_not_connected_line() -> CapturedEvent {
        let follow_up = FollowUp {
            tenant_id: Uuid::from_u128(4),
            user_id: Uuid::from_u128(5),
            backend: "sciotte_garmin".to_owned(),
            refused_at_ms: 1_764_000_000_000,
        };
        let last = LastRefusal {
            refused_at_ms: follow_up.refused_at_ms,
            reason: "Invalid password".to_owned(),
            method_used: "email".to_owned(),
            account: "j***@e***.com".to_owned(),
            refusals: 3,
        };
        let routes = serving_backends("garmin");
        let ((), events) = capture_events_while(|| {
            log_outcome(&follow_up, &last, &routes, 10, None);
        });
        assert_eq!(events.len(), 1, "one line per follow-up: {events:#?}");
        events.into_iter().next().unwrap() // Safe: length asserted above
    }

    #[test]
    fn a_follow_up_still_not_connected_is_a_warn_naming_what_ops_needs() {
        let event = still_not_connected_line();
        assert_eq!(event.level, Level::WARN, "the alert reads a WARN line");
        assert_eq!(event.target, FOLLOW_UP_TARGET);
        assert_eq!(event.message(), Some(STILL_NOT_CONNECTED));
        assert_eq!(
            event.field("provider"),
            Some("garmin"),
            "the user-facing provider"
        );
        assert_eq!(event.field("backend"), Some("sciotte_garmin"));
        assert_eq!(event.field("refusals"), Some("3"));
        assert_eq!(event.field("minutes"), Some("10"));
        assert_eq!(event.field("account"), Some("j***@e***.com"));
        assert_eq!(
            event.field("routes_checked"),
            Some(serving_backends("garmin").join(",").as_str()),
            "every route that could have connected the athlete is named"
        );
    }

    #[test]
    fn a_recovered_follow_up_is_an_info_naming_the_route_that_connected() {
        let follow_up = FollowUp {
            tenant_id: Uuid::nil(),
            user_id: Uuid::from_u128(5),
            backend: "sciotte".to_owned(),
            refused_at_ms: 0,
        };
        let last = LastRefusal {
            refused_at_ms: 0,
            reason: String::new(),
            method_used: "google".to_owned(),
            account: String::new(),
            refusals: 0,
        };
        let ((), events) = capture_events_while(|| {
            log_outcome(
                &follow_up,
                &last,
                &serving_backends("strava"),
                11,
                Some("strava"),
            );
        });
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(
            event.level,
            Level::INFO,
            "a recovery posts nothing to Slack"
        );
        assert_eq!(event.message(), Some(RECOVERED));
        assert_eq!(event.field("route"), Some("strava"));
        assert_eq!(event.field("provider"), Some("strava"));
        assert_eq!(
            event.field("refusals"),
            Some("1"),
            "a kept refusal counts as one at least"
        );
    }

    // A parity check between the log line and the Cloud Monitoring alert that
    // matches it: nothing at runtime joins the two, so the alert's filter and
    // label extractors are read from the Terraform and compared with the
    // event this module emits, in both directions.
    #[test]
    fn the_ops_alert_matches_the_follow_up_line_and_extracts_every_field_on_it() {
        let tf_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infra/environments/dev/provider_sign_in_monitoring.tf");
        let tf = fs::read_to_string(&tf_path)
            .unwrap_or_else(|e| panic!("the alert lives at {}: {e}", tf_path.display()));
        let event = still_not_connected_line();

        assert!(
            tf.contains(&format!("labels.\"rust.target\"=\"{}\"", event.target)),
            "the alert filters on the line's target {}",
            event.target
        );
        let matched = tf
            .split("jsonPayload.message:\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the alert matches on the message text");
        let message = event.message().expect("the line has a message");
        assert!(
            message.contains(matched) && !matched.is_empty(),
            "the alert's {matched:?} matches the line {message:?}"
        );
        for field in event.fields.keys().filter(|key| *key != "message") {
            assert!(
                tf.contains(&format!("EXTRACT(jsonPayload.{field})")),
                "the alert extracts the line's {field} field"
            );
        }
        for extracted in tf.split("EXTRACT(jsonPayload.").skip(1) {
            let field = extracted.split(')').next().unwrap_or_default();
            assert!(
                event.field(field).is_some(),
                "the alert extracts {field}, which the line does not carry"
            );
        }
    }
}
