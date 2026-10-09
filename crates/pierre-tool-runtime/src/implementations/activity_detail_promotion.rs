// ABOUTME: Detail promotion for get_activities — each listed activity's summary replaced by its detail read
// ABOUTME: Rationed by the provider's detail budget, and stopped once a read is refused for rate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_activities` promotes a small result set to detailed by reading each
//! activity's detail. Every activity is still returned; this module decides
//! which ones are read, and stops reading once the provider refuses for rate.

use pierre_core::errors::ErrorCode;
use pierre_core::models::Activity;
use pierre_providers::CoreFitnessProvider;
use tracing::warn;

/// `summaries` with each of the first `detail_budget` that `provider` recorded
/// (it answers to any of `provider_names`) replaced by its detail read, the
/// summary's merged-in fields kept. Every activity is returned: one past the
/// budget, one another provider recorded, and one whose read fails keep their
/// summaries, and once a read is refused for rate (the request budget or the
/// provider's own 429) the rest keep theirs without asking again, since every
/// further read would be refused the same way.
pub async fn promote_to_detail(
    provider: &dyn CoreFitnessProvider,
    summaries: &[Activity],
    detail_budget: usize,
    provider_names: &[&str],
) -> Vec<Activity> {
    let mut detailed = Vec::with_capacity(summaries.len());
    let mut requests_spent = false;
    for (rank, activity) in summaries.iter().enumerate() {
        // Past the budget: keep the summary. Rationing, not truncation —
        // every activity is still returned. A merged-in row from another
        // provider cannot be detailed through this provider's client — its id
        // would 404 (or worse, collide) — so it keeps its summary too.
        if rank >= detail_budget || requests_spent || !provider_names.contains(&activity.provider())
        {
            detailed.push(activity.clone());
            continue;
        }
        match provider.get_activity_detailed(activity.id()).await {
            // The summary may carry fields the merge took from another
            // recording of the session; the detail row keeps its own values
            // and gains those.
            Ok(mut detail) => {
                detail.fill_missing_from(activity);
                detailed.push(detail);
            }
            Err(err) => {
                requests_spent = err.code == ErrorCode::ExternalRateLimited;
                warn!(
                    activity_id = %activity.id(),
                    error = %err,
                    requests_spent,
                    "Detail fetch failed — retaining summary for this activity"
                );
                detailed.push(activity.clone());
            }
        }
    }
    detailed
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use pierre_core::errors::{AppError, AppResult, ErrorCode};
    use pierre_core::models::{Activity, ActivityBuilder, Athlete, SportType, Stats};
    use pierre_providers::core::{ActivityQueryParams, OAuth2Credentials, ProviderConfig};
    use pierre_providers::pagination::{CursorPage, PaginationParams};
    use pierre_providers::CoreFitnessProvider;

    use super::promote_to_detail;

    /// The provider the stand-in answers as.
    const PROVIDER: &str = "intervals_icu";

    /// A provider whose detail reads all fail with `code`, recording the id of
    /// each one it is asked for.
    struct RefusingDetail {
        config: ProviderConfig,
        code: ErrorCode,
        asked: Mutex<Vec<String>>,
    }

    impl RefusingDetail {
        fn failing_with(code: ErrorCode) -> Self {
            Self {
                config: ProviderConfig {
                    name: PROVIDER.to_owned(),
                    auth_url: String::new(),
                    token_url: String::new(),
                    api_base_url: String::new(),
                    revoke_url: None,
                    default_scopes: Vec::new(),
                },
                code,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CoreFitnessProvider for RefusingDetail {
        fn name(&self) -> &'static str {
            PROVIDER
        }

        fn config(&self) -> &ProviderConfig {
            &self.config
        }

        async fn set_credentials(&self, _: OAuth2Credentials) -> AppResult<()> {
            Ok(())
        }

        async fn is_authenticated(&self) -> bool {
            true
        }

        async fn refresh_token_if_needed(&self) -> AppResult<()> {
            Ok(())
        }

        async fn get_athlete(&self) -> AppResult<Athlete> {
            Err(AppError::internal("not read by this test"))
        }

        async fn get_activities_with_params(
            &self,
            _: &ActivityQueryParams,
        ) -> AppResult<Vec<Activity>> {
            Err(AppError::internal("not read by this test"))
        }

        async fn get_activities_cursor(
            &self,
            _: &PaginationParams,
        ) -> AppResult<CursorPage<Activity>> {
            Err(AppError::internal("not read by this test"))
        }

        async fn get_activity(&self, id: &str) -> AppResult<Activity> {
            self.asked.lock().unwrap().push(id.to_owned());
            Err(AppError::new(self.code, format!("detail of {id} refused")))
        }

        async fn get_stats(&self) -> AppResult<Stats> {
            Err(AppError::internal("not read by this test"))
        }
    }

    /// Three runs `PROVIDER` recorded, newest first.
    fn summaries() -> Vec<Activity> {
        (1..=3)
            .map(|n| {
                ActivityBuilder::new(
                    format!("a{n}"),
                    "Easy run",
                    SportType::Run,
                    Utc::now() - Duration::days(n),
                    1_800,
                    PROVIDER,
                )
                .build()
            })
            .collect()
    }

    fn ids(activities: &[Activity]) -> Vec<&str> {
        activities.iter().map(Activity::id).collect()
    }

    /// Once a detail read is refused for rate, the rest keep their summaries
    /// without being asked for: every activity is still returned, after one
    /// request.
    #[tokio::test]
    async fn a_rate_refusal_stops_the_detail_reads_that_follow_it() {
        let provider = RefusingDetail::failing_with(ErrorCode::ExternalRateLimited);
        let summaries = summaries();

        let served = promote_to_detail(&provider, &summaries, usize::MAX, &[PROVIDER]).await;

        assert_eq!(provider.asked(), vec!["a1"]);
        assert_eq!(ids(&served), vec!["a1", "a2", "a3"]);
    }

    /// Any other failed read costs only its own activity's detail: the next
    /// one is still asked for.
    #[tokio::test]
    async fn another_failure_leaves_the_next_detail_read_to_be_asked() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        let summaries = summaries();

        let served = promote_to_detail(&provider, &summaries, usize::MAX, &[PROVIDER]).await;

        assert_eq!(provider.asked(), vec!["a1", "a2", "a3"]);
        assert_eq!(ids(&served), vec!["a1", "a2", "a3"]);
    }
}
