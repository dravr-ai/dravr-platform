// ABOUTME: Detail promotion for get_activities — each listed activity's summary replaced by its detail read
// ABOUTME: Rationed by the provider's detail budget, and stopped once a read is refused for rate
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `get_activities` reads the detail of the activities the coach reasons
//! about: every one started in the last [`DETAIL_WINDOW_DAYS`] days, whatever
//! the call's `mode` or `limit`, and, when the call asked for
//! `mode=detailed`, the newest ones up to its cap. Every activity is still
//! returned; this module decides which ones carry their detail, takes a
//! detail already stored for one instead of reading it again, and stops
//! reading once the provider refuses for rate.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Utc};
use pierre_core::ai_policy::{AiUse, ProviderTerms};
use pierre_core::config::fitness::DETAIL_WINDOW_DAYS;
use pierre_core::errors::ErrorCode;
use pierre_core::models::Activity;
use pierre_providers::{ai_scope, CoreFitnessProvider};
use tracing::warn;

/// Which of a list's activities carry their detail, and how many provider
/// reads that may cost.
pub(crate) struct DetailPlan<'a> {
    /// Activities started at or after this instant are detailed.
    pub window_start: DateTime<Utc>,
    /// With `Some(n)`, the newest `n` activities are detailed too (an explicit
    /// `mode=detailed`).
    pub explicit_cap: Option<usize>,
    /// Most provider detail reads the call may make; stored details cost none.
    pub read_budget: usize,
    /// The provider names the client answers to: an activity another provider
    /// recorded cannot be read through it.
    pub provider_names: &'a [&'a str],
    /// The provider terms the read crosses: an activity whose terms let a
    /// model see only that it exists keeps its summary, since its detail would
    /// be withheld whole on the way out.
    pub terms: Option<&'a dyn ProviderTerms>,
}

impl DetailPlan<'_> {
    /// The window start `DETAIL_WINDOW_DAYS` before `now`.
    pub(crate) fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
        now - Duration::days(DETAIL_WINDOW_DAYS)
    }

    fn wants(&self, rank: usize, activity: &Activity) -> bool {
        self.provider_names.contains(&activity.provider())
            && self.detail_is_readable(activity)
            && (activity.start_date() >= self.window_start
                || self.explicit_cap.is_some_and(|cap| rank < cap))
    }
}

impl DetailPlan<'_> {
    /// Whether a model may read this activity's content at all: an
    /// existence-only or denied item never gains a detail it cannot show.
    fn detail_is_readable(&self, activity: &Activity) -> bool {
        let Some(terms) = self.terms.filter(|_| ai_scope::policies_apply()) else {
            return true;
        };
        terms
            .ai_policy(activity.provider())
            .map(|policy| policy.rule_for(activity.provider(), activity.source()))
            .is_none_or(|rule| matches!(rule, AiUse::Allow | AiUse::Redact(_)))
    }
}

/// What promotion produced: every activity in its original order, and the
/// details freshly read from the provider, for the caller to store so the
/// next call reads them from the cache.
pub(crate) struct Promoted {
    /// Every activity, the wanted ones with their detail where it was had.
    pub activities: Vec<Activity>,
    /// The details read from the provider by this call.
    pub fresh: Vec<Activity>,
    /// The ids of the activities that carry their detail.
    pub detailed_ids: HashSet<String>,
}

/// `summaries` with each activity the plan wants replaced by its detail.
///
/// A detail found in `stored` (keyed by provider and activity id) is used as it is and
/// costs no read. Otherwise `provider` is asked (with `None`, nothing is
/// read: a cached list is detailed from the store alone), within the plan's read
/// budget. The summary's merged-in fields are kept either way. An activity
/// past the budget, one another provider recorded, and one whose read fails
/// keep their summaries, and once a read is refused for rate (the request
/// budget or the provider's own 429) the rest keep theirs without asking
/// again, since every further read would be refused the same way.
pub(crate) async fn promote_to_detail(
    provider: Option<&dyn CoreFitnessProvider>,
    summaries: &[Activity],
    plan: &DetailPlan<'_>,
    stored: &HashMap<(String, String), Activity>,
) -> Promoted {
    let mut activities = Vec::with_capacity(summaries.len());
    let mut fresh = Vec::new();
    let mut detailed_ids = HashSet::new();
    let mut reads = 0_usize;
    let mut requests_spent = false;
    for (rank, activity) in summaries.iter().enumerate() {
        if !plan.wants(rank, activity) {
            activities.push(activity.clone());
            continue;
        }
        let key = (activity.provider().to_owned(), activity.id().to_owned());
        if let Some(held) = stored.get(&key) {
            let mut detail = held.clone();
            detail.fill_missing_from(activity);
            detailed_ids.insert(activity.id().to_owned());
            activities.push(detail);
            continue;
        }
        let Some(provider) = provider.filter(|_| !requests_spent && reads < plan.read_budget)
        else {
            activities.push(activity.clone());
            continue;
        };
        reads += 1;
        // The list read already tallied what the provider terms withheld of
        // this activity; its detail read tallies in a scope of its own, so
        // the same item is never counted twice.
        let (read, _) = ai_scope::tallied(provider.get_activity_detailed(activity.id())).await;
        match read {
            // The summary may carry fields the merge took from another
            // recording of the session; the detail row keeps its own values
            // and gains those.
            Ok(mut detail) => {
                fresh.push(detail.clone());
                detail.fill_missing_from(activity);
                detailed_ids.insert(activity.id().to_owned());
                activities.push(detail);
            }
            Err(err) => {
                requests_spent = err.code == ErrorCode::ExternalRateLimited;
                warn!(
                    activity_id = %activity.id(),
                    error = %err,
                    requests_spent,
                    "Detail fetch failed — retaining summary for this activity"
                );
                activities.push(activity.clone());
            }
        }
    }
    Promoted {
        activities,
        fresh,
        detailed_ids,
    }
}

/// The oldest start a stored-detail lookup for `activities` must reach: the
/// detail window, or the oldest row an explicit `mode=detailed` details.
pub(crate) fn detail_lookup_since(activities: &[Activity], plan: &DetailPlan<'_>) -> DateTime<Utc> {
    plan.explicit_cap
        .and_then(|cap| activities.iter().take(cap).map(Activity::start_date).min())
        .map_or(plan.window_start, |oldest| oldest.min(plan.window_start))
}

/// The mode a served list reports: `detailed` when every row carries its
/// detail, `summary` when none does, and the call's own mode otherwise (the
/// rows render one by one).
pub(crate) fn effective_mode<'m>(
    mode: &'m str,
    activities: &[Activity],
    detailed_ids: &HashSet<String>,
) -> &'m str {
    if !activities.is_empty() && activities.iter().all(|a| detailed_ids.contains(a.id())) {
        "detailed"
    } else if detailed_ids.is_empty() {
        "summary"
    } else {
        mode
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use pierre_core::errors::{AppError, AppResult, ErrorCode};
    use pierre_core::models::{Activity, ActivityBuilder, Athlete, SportType, Stats};
    use pierre_providers::core::{ActivityQueryParams, OAuth2Credentials, ProviderConfig};
    use pierre_providers::pagination::{CursorPage, PaginationParams};
    use pierre_providers::CoreFitnessProvider;

    use super::{promote_to_detail, DetailPlan};

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

        let served = promote_to_detail(
            Some(&provider),
            &summaries,
            &plan(None, usize::MAX),
            &none(),
        )
        .await
        .activities;

        assert_eq!(
            provider.asked(),
            vec!["a1"],
            "bug: a rate refusal must stop the reads"
        );
        assert_eq!(
            ids(&served),
            vec!["a1", "a2", "a3"],
            "bug: every activity is returned"
        );
    }

    /// Any other failed read costs only its own activity's detail: the next
    /// one is still asked for.
    #[tokio::test]
    async fn another_failure_leaves_the_next_detail_read_to_be_asked() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        let summaries = summaries();

        let served = promote_to_detail(
            Some(&provider),
            &summaries,
            &plan(None, usize::MAX),
            &none(),
        )
        .await
        .activities;

        assert_eq!(
            provider.asked(),
            vec!["a1", "a2", "a3"],
            "bug: one failure stops nothing"
        );
        assert_eq!(
            ids(&served),
            vec!["a1", "a2", "a3"],
            "bug: every activity is returned"
        );
    }

    const PROVIDERS: &[&str] = &[PROVIDER];

    fn plan(explicit_cap: Option<usize>, read_budget: usize) -> DetailPlan<'static> {
        DetailPlan {
            window_start: DetailPlan::window_start(Utc::now()),
            explicit_cap,
            read_budget,
            provider_names: PROVIDERS,
            terms: None,
        }
    }

    fn none() -> HashMap<(String, String), Activity> {
        HashMap::new()
    }

    /// The three recent runs, then one 20 days old.
    fn with_an_old_run() -> Vec<Activity> {
        let mut list = summaries();
        list.push(
            ActivityBuilder::new(
                "old",
                "Long run",
                SportType::Run,
                Utc::now() - Duration::days(20),
                5_400,
                PROVIDER,
            )
            .build(),
        );
        list
    }

    /// Detail is decided by day: every activity of the last 14 days is read,
    /// whatever the limit, and an older one only when the call asked for it.
    #[tokio::test]
    async fn only_the_last_fourteen_days_are_detailed_by_default() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        promote_to_detail(
            Some(&provider),
            &with_an_old_run(),
            &plan(None, usize::MAX),
            &none(),
        )
        .await;
        assert_eq!(
            provider.asked(),
            vec!["a1", "a2", "a3"],
            "bug: a 20-day-old run must keep its summary unless detail was asked for"
        );

        let explicit = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        promote_to_detail(
            Some(&explicit),
            &with_an_old_run(),
            &plan(Some(30), usize::MAX),
            &none(),
        )
        .await;
        assert_eq!(
            explicit.asked(),
            vec!["a1", "a2", "a3", "old"],
            "bug: mode=detailed must detail older rows up to its cap"
        );
    }

    /// A detail already stored is served from the store: no provider read.
    #[tokio::test]
    async fn a_stored_detail_costs_no_read() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        let held = ActivityBuilder::new(
            "a1",
            "Easy run",
            SportType::Run,
            Utc::now() - Duration::days(1),
            1_800,
            PROVIDER,
        )
        .description("Mal aux bras".to_owned())
        .build();
        let stored = HashMap::from([((PROVIDER.to_owned(), "a1".to_owned()), held)]);

        let promoted = promote_to_detail(
            Some(&provider),
            &summaries(),
            &plan(None, usize::MAX),
            &stored,
        )
        .await;

        assert_eq!(
            provider.asked(),
            vec!["a2", "a3"],
            "bug: a1's stored detail was read again"
        );
        assert_eq!(
            promoted.activities[0].description(),
            Some("Mal aux bras"),
            "bug: the stored detail must reach the row"
        );
        assert!(
            promoted.fresh.is_empty(),
            "bug: only reads that succeeded are fresh"
        );
    }

    /// A stored detail answers only its own provider's activity: an id is
    /// unique only within its provider.
    #[tokio::test]
    async fn another_providers_stored_detail_of_the_same_id_is_not_used() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        let foreign = ActivityBuilder::new(
            "a1",
            "Ride",
            SportType::Ride,
            Utc::now() - Duration::days(1),
            3_600,
            "strava",
        )
        .description("not this run".to_owned())
        .build();
        let stored = HashMap::from([(("strava".to_owned(), "a1".to_owned()), foreign)]);

        let promoted = promote_to_detail(
            Some(&provider),
            &summaries(),
            &plan(None, usize::MAX),
            &stored,
        )
        .await;

        assert_eq!(
            provider.asked(),
            vec!["a1", "a2", "a3"],
            "bug: another provider's detail stood in for a1's read"
        );
        assert!(
            promoted.activities[0].description().is_none(),
            "bug: another provider's detail reached a1"
        );
    }

    /// The read budget bounds provider reads, never the list.
    #[tokio::test]
    async fn the_read_budget_bounds_reads_not_rows() {
        let provider = RefusingDetail::failing_with(ErrorCode::ResourceNotFound);
        let served = promote_to_detail(Some(&provider), &summaries(), &plan(None, 1), &none())
            .await
            .activities;
        assert_eq!(
            provider.asked(),
            vec!["a1"],
            "bug: the budget allows one read"
        );
        assert_eq!(
            ids(&served),
            vec!["a1", "a2", "a3"],
            "bug: every activity is returned"
        );
    }
}
