// ABOUTME: Elects the connection that answers an athlete's activity questions — health, capability, then recency
// ABOUTME: A healthy provider that only detects workouts (WHOOP) never outranks a healthy recording source
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity-source election.
//!
//! When a tool asks for activities without naming a provider, one connection
//! answers. The repository ranks the athlete's connections by health and
//! recency (`ProviderConnectionRepository::rank_for_election`), but recency
//! is not fitness for the question: on 2026-08-22 a WHOOP connection added
//! after Strava was elected because it was newest, and served a 200 km ride
//! as a distance-less "run".
//!
//! What a provider can serve is declared on its descriptor
//! ([`ProviderCapabilities`]), so the election reads it from there: a
//! provider whose activities are recordings
//! ([`ProviderCapabilities::RECORDED_ACTIVITIES`]) outranks one that only
//! detects workouts ([`ProviderCapabilities::ACTIVITIES`] alone), which
//! outranks one that serves no activities at all. Within a rank the
//! repository's recency order stands.
//!
//! The full key, in order:
//!
//! 1. **Health.** A connection needing re-auth ranks after every `active`
//!    one, as the repository orders it. Capability does not override this:
//!    `get_activities` never serves a dead primary's own cache (its stand-in
//!    path asks the *siblings*), so electing a dead recorder over a healthy
//!    detector blanks a turn whose rides sit in the cache, while electing the
//!    detector lets the merge fold the flagged recorder's cached rows back in
//!    and keep their distance (`get_activities_dead_primary_test`).
//! 2. **A coach account last**, as the repository orders it: it has no
//!    calendar of its own, so it serves the athlete less than any detector.
//! 3. **Capability** — [`ActivitySourceRank`].
//! 4. **Recency**: the repository's `last_used_at`, then `connected_at`.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConnectionStatus, ProviderAccountRole, ProviderConnection, TenantId};
use pierre_database::repositories::ProviderConnectionRepository;
use uuid::Uuid;

use crate::registry::ProviderRegistry;
use crate::spi::ProviderCapabilities;

/// How well a provider answers an activity question, best first.
///
/// Ordered so the derived `Ord` is the election order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ActivitySourceRank {
    /// Its activities are the recordings: sport, distance and route as
    /// captured. Also the rank of a provider no descriptor describes — the
    /// election demotes only what a descriptor positively declares weaker.
    Recorded,
    /// It serves workouts it detected rather than recorded.
    Detected,
    /// It serves no activities.
    NoActivities,
}

impl ActivitySourceRank {
    /// The rank a descriptor's capabilities earn.
    #[must_use]
    pub const fn from_capabilities(capabilities: ProviderCapabilities) -> Self {
        if capabilities.supports_recorded_activities() {
            Self::Recorded
        } else if capabilities.supports_activities() {
            Self::Detected
        } else {
            Self::NoActivities
        }
    }
}

impl ProviderRegistry {
    /// The activity-source rank of the provider a connection is stored under.
    ///
    /// Resolved like the terms are: the descriptor registered under `provider`,
    /// else the one whose backend reads that service (a `garmin` connection
    /// in a build without the Garmin OAuth backend answers to the scraper that
    /// reads Garmin). A provider no descriptor describes is not demoted.
    #[must_use]
    pub fn activity_source_rank(&self, provider: &str) -> ActivitySourceRank {
        self.terms_descriptor(provider)
            .map_or(ActivitySourceRank::Recorded, |descriptor| {
                ActivitySourceRank::from_capabilities(descriptor.capabilities())
            })
    }
}

/// The connection that answers activity questions, from connections already
/// in the repository's election order.
///
/// Keeps the repository's health and coach-account keys, places
/// [`ActivitySourceRank`] after them, and lets recency break what remains
/// (see the module docs for why capability does not outrank health). `None`
/// only when there are no connections: an athlete whose sole connection
/// detects workouts still gets it, since it is the only answer there is.
#[must_use]
pub fn elect_activity_source(
    registry: &ProviderRegistry,
    ranked: Vec<ProviderConnection>,
) -> Option<ProviderConnection> {
    // `min_by_key` returns the FIRST minimum, which is what keeps the
    // repository's recency order among connections equal on this key.
    ranked.into_iter().min_by_key(|connection| {
        (
            connection.status != ConnectionStatus::Active,
            connection.account_role == Some(ProviderAccountRole::Coach),
            registry.activity_source_rank(&connection.provider),
        )
    })
}

/// Elect the connection that answers this athlete's activity questions.
///
/// The one election every caller choosing a primary provider goes through:
/// the repository's order with capability placed after health
/// ([`elect_activity_source`]). Tenant scope as the repository honours it.
///
/// # Errors
///
/// Propagates a repository failure.
pub async fn resolve_activity_source(
    connections: &dyn ProviderConnectionRepository,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: Option<TenantId>,
) -> AppResult<Option<ProviderConnection>> {
    let ranked = connections.rank_for_election(user_id, tenant_id).await?;
    Ok(elect_activity_source(registry, ranked))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use pierre_core::models::ConnectionType;

    fn connection(provider: &str) -> ProviderConnection {
        ProviderConnection {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::nil(),
            tenant_id: Uuid::nil().to_string(),
            provider: provider.to_owned(),
            connection_type: ConnectionType::OAuth,
            connected_at: Utc::now(),
            last_used_at: None,
            status: ConnectionStatus::Active,
            metadata: None,
            account_role: None,
        }
    }

    #[test]
    fn ranks_follow_the_declared_capabilities() {
        assert_eq!(
            ActivitySourceRank::from_capabilities(ProviderCapabilities::activity_only()),
            ActivitySourceRank::Recorded
        );
        assert_eq!(
            ActivitySourceRank::from_capabilities(
                ProviderCapabilities::OAUTH.union(ProviderCapabilities::ACTIVITIES)
            ),
            ActivitySourceRank::Detected
        );
        assert_eq!(
            ActivitySourceRank::from_capabilities(
                ProviderCapabilities::OAUTH.union(ProviderCapabilities::SLEEP_TRACKING)
            ),
            ActivitySourceRank::NoActivities
        );
    }

    #[cfg(all(feature = "provider-whoop", feature = "provider-strava"))]
    #[test]
    fn a_detector_ranked_first_loses_to_a_recording_source() {
        let registry = ProviderRegistry::new();
        assert_eq!(
            registry.activity_source_rank("whoop"),
            ActivitySourceRank::Detected
        );
        let elected =
            elect_activity_source(&registry, vec![connection("whoop"), connection("strava")])
                .expect("two connections");
        assert_eq!(elected.provider, "strava");
    }

    #[cfg(all(feature = "provider-whoop", feature = "provider-strava"))]
    #[test]
    fn health_and_the_coach_key_still_rank_ahead_of_capability() {
        let registry = ProviderRegistry::new();
        let mut dead_strava = connection("strava");
        dead_strava.status = ConnectionStatus::NeedsReauth;
        let elected = elect_activity_source(&registry, vec![connection("whoop"), dead_strava])
            .expect("two connections");
        assert_eq!(
            elected.provider, "whoop",
            "a dead recorder never shadows a healthy detector"
        );

        let mut coach_strava = connection("strava");
        coach_strava.account_role = Some(ProviderAccountRole::Coach);
        let elected = elect_activity_source(&registry, vec![connection("whoop"), coach_strava])
            .expect("two connections");
        assert_eq!(
            elected.provider, "whoop",
            "a coach account serves the athlete least"
        );
    }

    #[cfg(all(feature = "provider-whoop", feature = "provider-strava"))]
    #[test]
    fn equal_ranks_keep_the_repository_order_and_a_lone_detector_still_answers() {
        let registry = ProviderRegistry::new();
        let elected = elect_activity_source(
            &registry,
            vec![
                connection("whoop"),
                connection("unknown_backend"),
                connection("strava"),
            ],
        )
        .expect("three connections");
        assert_eq!(
            elected.provider, "unknown_backend",
            "an undescribed provider is not demoted, and ties keep the input order"
        );
        let lone =
            elect_activity_source(&registry, vec![connection("whoop")]).expect("one connection");
        assert_eq!(lone.provider, "whoop");
        assert!(elect_activity_source(&registry, Vec::new()).is_none());
    }
}
