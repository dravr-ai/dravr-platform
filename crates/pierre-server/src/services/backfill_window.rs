// ABOUTME: The warmed window a backfill-completion notice lists, read as a model may see it
// ABOUTME: Applies each provider's terms to the cached activities the backfill just wrote

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity window behind a backfill-completion notice (carnet#734).
//!
//! The notice's list lands in a conversation a model reads back, so the window
//! is read for a model, served over the surface the notice is delivered to:
//! each provider's terms decide which sessions, and which of their fields,
//! the list may name.

use chrono::{Duration, TimeZone, Utc};
use pierre_core::ai_policy::{ProviderTerms, Withheld};
use pierre_core::models::{Activity, TenantId};
use pierre_core::transport::Transport;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use tracing::warn;
use uuid::Uuid;

/// Read limit for the warmed window — covers a deep backfill while staying well
/// above the notice's inline list cap so the rendered list and the "and N more"
/// count reflect the true window size, not a truncated read.
const BACKFILL_WINDOW_READ_LIMIT: i64 = 500;

/// Lower bound (days) used when the job carries no `after` timestamp, so the
/// window read still has a finite floor. The backfill always passes the job's
/// `after`, so this is only a defensive fallback.
const BACKFILL_FALLBACK_WINDOW_DAYS: i64 = 90;

/// Read the warmed window's cached activities straight from the durable
/// activity cache the backfill just populated, as a model may see them.
///
/// Reads through the `ActivityCacheRepository` on `repos` — no `ToolRuntime`
/// dependency is pulled into the notifier. The window mirrors the job:
/// `[after_ts, now]`, newest first, capped at [`BACKFILL_WINDOW_READ_LIMIT`].
/// Returns empty (then the caller falls back to the templated nudge) when the
/// cache read misses or fails.
///
/// The read is a read for a model served over `transport`, the surface the
/// notice is delivered to; what the provider terms held back is returned
/// beside the activities.
pub async fn read_warmed_window(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
    after_ts: i64,
    transport: Transport,
) -> (Vec<Activity>, Withheld) {
    let now = Utc::now();
    let start = Utc
        .timestamp_opt(after_ts, 0)
        .single()
        .unwrap_or_else(|| now - Duration::days(BACKFILL_FALLBACK_WINDOW_DAYS));
    let read = async {
        match repos
            .activity_cache
            .get_cached_activities(
                user_id,
                &tenant_id,
                Some(provider),
                start,
                now,
                BACKFILL_WINDOW_READ_LIMIT,
            )
            .await
        {
            Ok(activities) => ai_scope::filter_activities(terms, activities),
            Err(e) => {
                warn!(error = %e, provider = %provider, "Backfill push: warmed-window cache read failed");
                Vec::new()
            }
        }
    };
    ai_scope::serve_over(transport, ai_scope::ai_read(read)).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::models::{ActivityBuilder, SportType};
    use pierre_core::transport::TransportPolicy;
    use pierre_providers::provider_terms::{NOLIO, NOLIO_TRANSPORT};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    use super::*;

    const RELAY: &str = "nolio";

    /// Terms that know only the Nolio relay: garmin allowed, strava existence
    /// only, zepp denied.
    struct NolioTerms;

    impl ProviderTerms for NolioTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == RELAY).then_some(&NOLIO)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == RELAY).then_some(NOLIO_TRANSPORT)
        }
    }

    fn relayed(id: &str, name: &str, source: &str, days_ago: i64) -> Activity {
        ActivityBuilder::new(
            id,
            name,
            SportType::Run,
            Utc::now() - Duration::days(days_ago),
            3_600,
            RELAY,
        )
        .distance_meters(10_000.0)
        .source(source)
        .build()
    }

    #[tokio::test]
    async fn the_window_names_no_session_the_terms_withhold_from_a_model() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("{}@relay.test", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant_id = TenantId::parse_str(&Uuid::new_v4().to_string()).unwrap();
        repos
            .activity_cache
            .upsert_activities(
                user_id,
                &tenant_id,
                RELAY,
                &[
                    relayed("g", "Garmin tempo", "garmin", 3),
                    relayed("s", "Strava secret name", "strava", 2),
                    relayed("z", "Secret zepp ride", "zepp", 1),
                ],
            )
            .await
            .unwrap();

        let after = (Utc::now() - Duration::days(10)).timestamp();
        let (warmed, withheld) = read_warmed_window(
            &repos,
            &NolioTerms,
            user_id,
            tenant_id,
            RELAY,
            after,
            Transport::Messaging,
        )
        .await;
        let names: Vec<&str> = warmed.iter().map(Activity::name).collect();

        assert!(names.contains(&"Garmin tempo"), "{names:?}");
        assert!(
            !names.contains(&"Strava secret name"),
            "existence only: {names:?}"
        );
        assert!(!names.contains(&"Secret zepp ride"), "denied: {names:?}");
        assert_eq!(warmed.len(), 2, "the denied session is not listed at all");
        assert_eq!((withheld.dropped, withheld.reduced), (1, 1));
    }
}
