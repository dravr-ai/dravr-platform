// ABOUTME: Serves an activity read the athlete's session-read cap refused: their Strava grant, else the stored copy
// ABOUTME: The copy carries a read_limit caveat; with nothing stored the refusal stands (carnet#1661)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The stored-copy answer to a capped session read (carnet#1661).
//!
//! The tools that read an athlete's activities straight from their provider
//! (the analytics, the goals) answer a read their session-read cap refused
//! here, in one order:
//!
//! 1. the athlete's Strava grant, when the capped session is their Strava
//!    session ([`capped_session_falls_back`]): the grant has a request
//!    budget of its own and costs the session nothing. A by-id read already asked it, inside the provider handle
//!    (`protocol::auth::grant_by_id`);
//! 2. the activities the platform stored for that backend, under a
//!    `provider_unavailable` caveat whose `reason` is `read_limit`, saying
//!    when live reads resume;
//! 3. with nothing stored, the refusal itself, which carries the wait.
//!
//! Any other failure is returned as it is: only the cap's refusal was refused
//! before it reached the provider, so only it says nothing of the provider.

use std::fmt;
use std::sync::Arc;

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use async_trait::async_trait;
use chrono::{DateTime, Utc};
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use pierre_core::constants::oauth_providers;
#[cfg(feature = "tools-analytics")]
use pierre_core::errors::protocol::ProtocolError;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::ai_scope;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use pierre_providers::backend_resolver;
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::core::OwnerStores;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use pierre_providers::core::{ActivityQueryParams, FitnessProvider};
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use pierre_providers::request_budget::session_read_cap;
use pierre_services::provider_refresh::UNREAD_PROVIDER_DIRECTIVE;
use pierre_services::training_history_read::history_store_keys;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use pierre_tools_core::ToolResult;
use serde_json::{json, Value};
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use tracing::debug;
use uuid::Uuid;

use super::data_helpers::read_limit_note;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use crate::activity_fetch::read_cached_window;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use crate::activity_fetch::sync_verdict::capped_session_falls_back;
use crate::activity_fetch::StaleReason;
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
use crate::protocol::auth::AuthService;
#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
use crate::protocol::types::UniversalResponse;
use crate::runtime::ToolRuntime;

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// What a read answered, and the caveat a stored copy standing in for it
/// carries.
#[derive(Debug)]
pub(crate) struct Served<T> {
    /// What was read: live, through the grant, or the stored copy.
    pub(crate) value: T,
    /// The `provider_unavailable` caveat of a stored copy; `None` for a live
    /// answer.
    pub(crate) caveat: Option<Value>,
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
impl<T> Served<T> {
    /// A live answer, with no caveat.
    pub(crate) const fn live(value: T) -> Self {
        Self {
            value,
            caveat: None,
        }
    }

    /// The same answer with its value turned by `f`; the caveat stays.
    pub(crate) fn map<U>(self, f: impl FnOnce(T) -> U) -> Served<U> {
        Served {
            value: f(self.value),
            caveat: self.caveat,
        }
    }
}

/// Whose read it is: the athlete and the tenant their rows are stored in.
#[derive(Clone, Copy)]
pub(crate) struct Reader<'run> {
    /// The runtime whose repositories and grants serve the read.
    pub(crate) runtime: &'run Arc<dyn ToolRuntime>,
    /// The athlete.
    pub(crate) user_id: Uuid,
    /// The tenant of the request; `None` stores nothing to serve.
    pub(crate) tenant_id: Option<&'run str>,
}

impl fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("user_id", &self.user_id)
            .field("tenant_id", &self.tenant_id)
            .finish_non_exhaustive()
    }
}

impl<'run> Reader<'run> {
    /// `user_id`'s read, in the request's `tenant_id`, served through `runtime`.
    #[must_use]
    pub(crate) const fn new(
        runtime: &'run Arc<dyn ToolRuntime>,
        user_id: Uuid,
        tenant_id: Option<&'run str>,
    ) -> Self {
        Self {
            runtime,
            user_id,
            tenant_id,
        }
    }

    /// The request's tenant, when it names one.
    fn tenant(&self) -> Option<TenantId> {
        self.tenant_id.and_then(|t| TenantId::parse_str(t).ok())
    }
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// `provider`'s `limit` most recent activities, or what stands in for them.
///
/// When the athlete's session-read cap refused the read: their Strava
/// grant's, else the stored ones under a `read_limit` caveat, else the
/// refusal.
///
/// # Errors
///
/// The read's own error, or the cap's refusal when nothing answers in its
/// place.
pub(crate) async fn activities_or_stored(
    reader: Reader<'_>,
    provider: &dyn FitnessProvider,
    limit: usize,
) -> AppResult<Served<Vec<Activity>>> {
    let refusal = match LiveRead::of(provider.get_activities(Some(limit), None).await) {
        LiveRead::Capped(refusal) => refusal,
        LiveRead::Served(read) => return read,
    };
    match reader.tenant() {
        Some(tenant) => {
            let store = TenantStore { reader, tenant };
            activities_in_place(&store, refusal, provider.name(), limit).await
        }
        None => Err(refusal),
    }
}

#[cfg(feature = "tools-analytics")]
/// `provider`'s activity `activity_id`, or what stands in for it.
///
/// When the athlete's session-read cap refused the read, and the Strava
/// grant the provider handle asks first did not answer: its stored row under
/// a `read_limit` caveat, else the refusal.
///
/// # Errors
///
/// The read's own error, or the cap's refusal when nothing is stored.
pub(crate) async fn activity_or_stored(
    reader: Reader<'_>,
    provider: &dyn FitnessProvider,
    activity_id: &str,
) -> AppResult<Served<Activity>> {
    let refusal = match LiveRead::of(provider.get_activity(activity_id).await) {
        LiveRead::Capped(refusal) => refusal,
        LiveRead::Served(read) => return read,
    };
    match reader.tenant() {
        Some(tenant) => {
            let store = TenantStore { reader, tenant };
            activity_in_place(&store, refusal, provider.name(), activity_id).await
        }
        None => Err(refusal),
    }
}

/// The activity `activity_id` of `backend` with its stored samples, standing
/// in for a by-id sample read the athlete's session-read cap refused
/// (carnet#1661), else `refusal`.
///
/// The Strava grant the provider handle asks first did not answer by then
/// (`protocol::auth::grant_by_id`). Only a copy whose samples were stored
/// stands in: the sample tools (`export_intervals`, `export_routes`,
/// `extract_activity_streams`) answer from them, and a copy without them would
/// read as an activity that recorded none. A recorded activity's samples do
/// not change, so the copy answers what the live read would have.
///
/// # Errors
///
/// `refusal`, when nothing stored stands in.
pub(crate) async fn samples_or_stored(
    reader: Reader<'_>,
    backend: &str,
    activity_id: &str,
    refusal: AppError,
) -> AppResult<Activity> {
    let Some(tenant) = reader.tenant() else {
        return Err(refusal);
    };
    let stores = OwnerStores::from_repos(reader.runtime.repos(), reader.user_id, tenant);
    // Its own key, then the other half of its pair ([`history_store_keys`]).
    for key in history_store_keys(backend) {
        if !matches!(
            stores.stored_activity_streams(key, activity_id).await,
            Ok(Some(_))
        ) {
            continue;
        }
        let Ok(activity) = stores
            .stored_activity_with_streams(key, activity_id, "activity")
            .await
        else {
            continue;
        };
        // As a model may see it, as a live read would be.
        let registry = reader.runtime.provider_registry();
        if let Some(activity) = ai_scope::filter_activities(registry.as_ref(), vec![activity])
            .into_iter()
            .next()
        {
            return Ok(activity);
        }
    }
    Err(refusal)
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// How a live read came out: served as it is, or refused by the athlete's
/// session-read cap.
enum LiveRead<T> {
    /// Its answer or any failure but the cap's, which stands as it is.
    Served(AppResult<Served<T>>),
    /// The cap's refusal: something may answer in its place.
    Capped(AppError),
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
impl<T> LiveRead<T> {
    /// What `read` came to.
    fn of(read: AppResult<T>) -> Self {
        match read {
            Err(e) if session_read_cap(&e).is_some() => Self::Capped(e),
            read => Self::Served(read.map(Served::live)),
        }
    }
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// What answers a capped read in its place: the grant behind the session,
/// and what the platform stored for the athlete.
#[async_trait]
trait StandIn: Send + Sync {
    /// The Strava grant's `limit` most recent activities, when the capped
    /// read of `backend` asks it ([`capped_session_falls_back`]) and it
    /// answers.
    async fn grant_activities(
        &self,
        refusal: &AppError,
        backend: &str,
        limit: usize,
    ) -> Option<Vec<Activity>>;

    /// The `limit` most recent activities stored for `backend`, however
    /// old; `None` when none is.
    async fn stored_activities(&self, backend: &str, limit: usize) -> Option<Vec<Activity>>;

    #[cfg(feature = "tools-analytics")]
    /// The stored row of `activity_id` for `backend`.
    async fn stored_activity(&self, backend: &str, activity_id: &str) -> Option<Activity>;

    /// When `backend` last synced for the athlete.
    async fn synced_at(&self, backend: &str) -> Option<DateTime<Utc>>;
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// [`activities_or_stored`] once the cap refused the read with `refusal`:
/// the grant, else the stored copy under its caveat, else the refusal.
async fn activities_in_place(
    store: &impl StandIn,
    refusal: AppError,
    backend: &str,
    limit: usize,
) -> AppResult<Served<Vec<Activity>>> {
    if let Some(rows) = store.grant_activities(&refusal, backend, limit).await {
        return Ok(Served::live(rows));
    }
    let Some(rows) = store.stored_activities(backend, limit).await else {
        return Err(refusal);
    };
    let synced_at = store.synced_at(backend).await;
    Ok(Served {
        value: rows,
        caveat: Some(caveat(
            backend,
            "activities",
            StaleReason::of_failure(&refusal),
            synced_at,
        )),
    })
}

#[cfg(feature = "tools-analytics")]
/// [`activity_or_stored`] once the cap refused the read with `refusal`: the
/// stored row under its caveat, else the refusal.
async fn activity_in_place(
    store: &impl StandIn,
    refusal: AppError,
    backend: &str,
    activity_id: &str,
) -> AppResult<Served<Activity>> {
    let Some(activity) = store.stored_activity(backend, activity_id).await else {
        return Err(refusal);
    };
    let synced_at = store.synced_at(backend).await;
    Ok(Served {
        value: activity,
        caveat: Some(caveat(
            backend,
            "activity",
            StaleReason::of_failure(&refusal),
            synced_at,
        )),
    })
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
/// The athlete's grant and stored rows in the request's tenant.
struct TenantStore<'run> {
    reader: Reader<'run>,
    tenant: TenantId,
}

#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
#[async_trait]
impl StandIn for TenantStore<'_> {
    async fn grant_activities(
        &self,
        refusal: &AppError,
        backend: &str,
        limit: usize,
    ) -> Option<Vec<Activity>> {
        if !capped_session_falls_back(refusal, backend) {
            return None;
        }
        let Reader {
            runtime, user_id, ..
        } = self.reader;
        let plan =
            backend_resolver::read_plan(runtime.repos(), user_id, self.tenant, backend).await;
        let grant = plan
            .fallback
            .filter(|grant| plan.primary == backend && grant == oauth_providers::STRAVA)?;
        let tenant_id = self.tenant.to_string();
        let provider = AuthService::new(Arc::clone(runtime))
            .create_exact_provider(&grant, user_id, Some(&tenant_id))
            .await
            .map_err(|response| {
                debug!(
                    user_id = %user_id,
                    grant = %grant,
                    cause = ?response.error,
                    "the grant behind a capped session could not be authenticated"
                );
            })
            .ok()?;
        match provider.get_activities(Some(limit), None).await {
            Ok(rows) => Some(rows),
            Err(e) => {
                debug!(
                    user_id = %user_id,
                    grant = %grant,
                    error = %e,
                    "the grant behind a capped session did not answer; serving the stored copy"
                );
                None
            }
        }
    }

    async fn stored_activities(&self, backend: &str, limit: usize) -> Option<Vec<Activity>> {
        // As many as the live read asked for, however old: it had no window.
        let params = ActivityQueryParams {
            limit: Some(limit),
            offset: None,
            before: None,
            after: Some(0),
        };
        let Reader {
            runtime, user_id, ..
        } = self.reader;
        read_cached_window(runtime, backend, user_id, self.tenant, &params).await
    }

    #[cfg(feature = "tools-analytics")]
    async fn stored_activity(&self, backend: &str, activity_id: &str) -> Option<Activity> {
        let Reader {
            runtime, user_id, ..
        } = self.reader;
        let cache = &runtime.repos().activity_cache;
        // Its own key, then the other half of its pair ([`history_store_keys`]).
        for key in history_store_keys(backend) {
            let Ok(Some(activity)) = cache
                .get_cached_activity(user_id, &self.tenant, key, activity_id)
                .await
            else {
                continue;
            };
            // As a model may see it, as a live read would be.
            return ai_scope::filter_activities(
                runtime.provider_registry().as_ref(),
                vec![activity],
            )
            .into_iter()
            .next();
        }
        None
    }

    async fn synced_at(&self, backend: &str) -> Option<DateTime<Utc>> {
        self.reader
            .runtime
            .repos()
            .activity_cache
            .latest_activity_sync(self.reader.user_id, &self.tenant, backend)
            .await
            .ok()
            .flatten()
    }
}

/// The `provider_unavailable` caveat for `what` served from a copy of `backend`.
///
/// `what` is a profile, statistics, an activity; the copy is the one
/// `backend` last answered at `synced_at` (`null` when the platform holds no
/// sync time for it), served because of `reason`. A
/// [`StaleReason::ReadLimit`] says when live reads resume, under
/// `retry_after_secs`.
#[must_use]
pub(crate) fn caveat(
    backend: &str,
    what: &str,
    reason: StaleReason,
    synced_at: Option<DateTime<Utc>>,
) -> Value {
    let name = user_facing_name(backend);
    let when = synced_at.map_or_else(
        || "last synced".to_owned(),
        |at| format!("last synced ({})", at.format("%Y-%m-%d %H:%M UTC")),
    );
    let note = reason.retry_after_secs().map_or_else(
        || {
            format!(
                "{name} could not be read just now, so the {what} above is as it was {when}: \
                 anything that changed since is missing, and its connection is intact, so do \
                 not ask the athlete to reconnect it. Answer from it. \
                 {UNREAD_PROVIDER_DIRECTIVE}"
            )
        },
        |retry_after_secs| read_limit_note(name, what, &when, retry_after_secs),
    );
    let mut caveat = json!({
        "provider": name,
        "reason": reason.as_str(),
        "last_synced_at": synced_at.map(|at| at.to_rfc3339()),
        "note": note,
    });
    if let (Some(retry_after_secs), Some(object)) =
        (reason.retry_after_secs(), caveat.as_object_mut())
    {
        object.insert("retry_after_secs".to_owned(), json!(retry_after_secs));
    }
    caveat
}

#[cfg(feature = "tools-analytics")]
/// `formatted`, carrying `caveat` when a stored copy answered.
///
/// # Errors
///
/// `formatted`'s own error.
pub(crate) fn with_caveat(
    formatted: Result<UniversalResponse, ProtocolError>,
    caveat: Option<Value>,
) -> Result<UniversalResponse, ProtocolError> {
    let mut response = formatted?;
    if let Some(caveat) = caveat {
        attach_caveat(&mut response, caveat);
    }
    Ok(response)
}

#[cfg(any(feature = "tools-data", feature = "tools-analytics"))]
/// Put `caveat` on a served response's payload.
pub(crate) fn attach_caveat(response: &mut UniversalResponse, caveat: Value) {
    if let Some(payload) = response.result.as_mut() {
        put_caveat(payload, caveat);
    }
}

/// `result`, carrying `caveat` when a stored copy answered: the tool-result
/// counterpart of [`with_caveat`].
///
/// # Errors
///
/// `result`'s own error.
#[cfg(any(feature = "tools-goals", feature = "tools-analytics"))]
pub(crate) fn caveated(
    result: AppResult<ToolResult>,
    caveat: Option<Value>,
) -> AppResult<ToolResult> {
    let mut result = result?;
    if let Some(caveat) = caveat {
        put_caveat(&mut result.content, caveat);
    }
    Ok(result)
}

#[cfg(any(
    feature = "tools-data",
    feature = "tools-goals",
    feature = "tools-analytics"
))]
/// Put `caveat` on `payload`, an object; any other payload has no place for it.
fn put_caveat(payload: &mut Value, caveat: Value) {
    if let Some(object) = payload.as_object_mut() {
        object.insert("provider_unavailable".to_owned(), caveat);
    }
}

#[cfg(test)]
#[cfg(feature = "tools-analytics")]
mod tests {
    use async_trait::async_trait;
    use chrono::{DateTime, TimeZone, Utc};
    use pierre_core::errors::{AppError, ErrorCode};
    use pierre_core::models::activity::ActivityBuilder;
    use pierre_core::models::{Activity, SportType};
    use serde_json::json;

    use super::{activities_in_place, activity_in_place, LiveRead, StandIn};

    /// A session-read cap's refusal of a read the athlete waits on, as the
    /// limiter marks it, retryable in ten minutes.
    fn capped() -> AppError {
        let mut error = AppError::new(
            ErrorCode::ExternalRateLimited,
            "Garmin has been read as often as Dravr allows for now",
        );
        error.details = Some(Box::new(json!({
            "refused_before_send": true,
            "retry_after_secs": 600,
            "session_read_cap": "asked",
        })));
        error
    }

    fn run(id: &str) -> Activity {
        ActivityBuilder::new(
            id,
            "Run",
            SportType::Run,
            Utc::now(),
            3_600,
            "sciotte_garmin",
        )
        .build()
    }

    fn synced() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 19, 8, 30, 0)
            .single()
            .unwrap_or_default()
    }

    /// The grant and the stored rows a test sets.
    #[derive(Default)]
    struct Fixed {
        grant: Option<Vec<Activity>>,
        stored: Option<Vec<Activity>>,
        stored_one: Option<Activity>,
    }

    #[async_trait]
    impl StandIn for Fixed {
        async fn grant_activities(&self, _: &AppError, _: &str, _: usize) -> Option<Vec<Activity>> {
            self.grant.clone()
        }

        async fn stored_activities(&self, _: &str, _: usize) -> Option<Vec<Activity>> {
            self.stored.clone()
        }

        async fn stored_activity(&self, _: &str, _: &str) -> Option<Activity> {
            self.stored_one.clone()
        }

        async fn synced_at(&self, _: &str) -> Option<DateTime<Utc>> {
            Some(synced())
        }
    }

    #[tokio::test]
    async fn a_capped_read_with_stored_rows_serves_them_under_a_read_limit_caveat() {
        let store = Fixed {
            stored: Some(vec![run("a"), run("b")]),
            ..Fixed::default()
        };
        let served = activities_in_place(&store, capped(), "sciotte_garmin", 50)
            .await
            .expect("the stored copy answers a capped read");
        assert_eq!(
            served.value.len(),
            2,
            "bug: the stored copy did not serve both stored runs"
        );
        let caveat = served.caveat.expect("a stored copy says so");
        assert_eq!(
            caveat["reason"], "read_limit",
            "bug: a capped read's copy is not tagged read_limit: {caveat}"
        );
        assert_eq!(
            caveat["retry_after_secs"], 600,
            "bug: the caveat does not carry the refusal's wait: {caveat}"
        );
        assert_eq!(
            caveat["provider"], "garmin",
            "bug: the caveat does not name the provider as the athlete knows it: {caveat}"
        );
        let note = caveat["note"].as_str().unwrap_or_default();
        assert!(
            note.contains("used up for now"),
            "bug: the note does not say live reads are used up: {note}"
        );
        assert!(
            note.contains("about 10 min"),
            "bug: the note does not say when live reads resume: {note}"
        );
        assert!(
            !note.contains("could not be read"),
            "bug: a capped read is worded as a failed one: {note}"
        );
    }

    #[tokio::test]
    async fn a_capped_read_with_nothing_stored_returns_the_refusal() {
        let refused = activities_in_place(&Fixed::default(), capped(), "sciotte_garmin", 50)
            .await
            .expect_err("nothing answers in its place");
        assert_eq!(
            refused.code,
            ErrorCode::ExternalRateLimited,
            "bug: with nothing stored, the cap's refusal does not stand"
        );
        assert_eq!(
            refused.retry_after_secs(),
            Some(600),
            "bug: the standing refusal lost its wait"
        );
        let refused = activity_in_place(&Fixed::default(), capped(), "sciotte_garmin", "a")
            .await
            .expect_err("no row is stored for the id");
        assert_eq!(
            refused.retry_after_secs(),
            Some(600),
            "bug: a by-id read with nothing stored lost the refusal's wait"
        );
    }

    #[tokio::test]
    async fn the_grant_answers_before_any_stored_copy() {
        let store = Fixed {
            grant: Some(vec![run("live")]),
            stored: Some(vec![run("stored")]),
            ..Fixed::default()
        };
        let served = activities_in_place(&store, capped(), "sciotte", 50)
            .await
            .expect("the grant answers");
        assert_eq!(
            served.value.first().map(Activity::id),
            Some("live"),
            "bug: the stored copy answered before the grant"
        );
        assert!(
            served.caveat.is_none(),
            "bug: the grant's live answer carries a stored copy's caveat"
        );
    }

    #[tokio::test]
    async fn a_capped_read_of_one_activity_serves_its_stored_row() {
        let store = Fixed {
            stored_one: Some(run("a")),
            ..Fixed::default()
        };
        let served = activity_in_place(&store, capped(), "sciotte_garmin", "a")
            .await
            .expect("the stored row answers");
        assert_eq!(
            served.value.id(),
            "a",
            "bug: the stored row of another id answered"
        );
        let caveat = served.caveat.expect("a stored row says so");
        assert_eq!(
            caveat["reason"], "read_limit",
            "bug: a capped by-id read's row is not tagged read_limit: {caveat}"
        );
        assert_eq!(
            caveat["last_synced_at"],
            synced().to_rfc3339(),
            "bug: the caveat does not say when the copy was synced: {caveat}"
        );
    }

    #[test]
    fn any_other_failure_and_any_answer_pass_through() {
        let failed: Result<Vec<Activity>, AppError> = Err(AppError::new(
            ErrorCode::ExternalServiceUnavailable,
            "the browser wedged",
        ));
        let LiveRead::Served(Err(e)) = LiveRead::of(failed) else {
            panic!("bug: a failure other than the cap's was answered in its place");
        };
        assert_eq!(
            e.code,
            ErrorCode::ExternalServiceUnavailable,
            "bug: another failure was rewritten on its way through"
        );
        let mut app_budget = AppError::new(ErrorCode::ExternalRateLimited, "strava budget spent");
        app_budget.details = Some(Box::new(json!({ "refused_before_send": true })));
        assert!(
            matches!(
                LiveRead::<Vec<Activity>>::of(Err(app_budget)),
                LiveRead::Served(Err(_))
            ),
            "bug: the app budget's refusal was taken for the athlete's cap"
        );
        assert!(
            matches!(
                LiveRead::of(Ok(vec![run("a")])),
                LiveRead::Served(Ok(served)) if served.caveat.is_none()
            ),
            "bug: a live answer was not served as it is, uncaveated"
        );
        assert!(
            matches!(
                LiveRead::<Vec<Activity>>::of(Err(capped())),
                LiveRead::Capped(_)
            ),
            "bug: the cap's refusal was not recognised as one"
        );
    }
}
