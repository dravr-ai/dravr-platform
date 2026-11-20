// ABOUTME: A provider read bounded by the refresh budget, falling back to its last-known copy with a caveat
// ABOUTME: A slow or failing provider serves what was last synced and says why and when, instead of failing the call
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Last-known fallback for live provider reads.
//!
//! `get_athlete` reads the provider live on a cache miss. A
//! provider that hangs held the whole turn, and one that failed failed the
//! call, though the platform had read the same profile an hour earlier. Here
//! the live read gets the refresh budget, every successful read is also kept
//! as a last-known copy for [`TTL_LAST_KNOWN_SECS`], and a read that runs out
//! of budget or fails serves that copy under a `provider_unavailable` caveat
//! carrying the `reason` and `last_synced_at`, the same sidecar a stale
//! activity window carries.
//!
//! A read that outlives the budget is not cancelled: it runs on in its own
//! task and writes both caches, so the next turn is current.
//!
//! [`TTL_LAST_KNOWN_SECS`]: pierre_cache::TTL_LAST_KNOWN_SECS

use std::collections::HashMap;
use std::future::Future;
use std::time::Duration;

#[cfg(feature = "tools-data")]
use chrono::{DateTime, Utc};
#[cfg(feature = "tools-data")]
use pierre_cache::{Cache, CacheKey};
use pierre_config::timeouts::ServerTimeouts;
use pierre_core::models::refresh::RefreshConfig;
use pierre_providers::request_budget::{keep_scope, session_read_cap};
#[cfg(feature = "tools-data")]
use serde::de::DeserializeOwned;
#[cfg(feature = "tools-data")]
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::timeout;
use tracing::{debug, warn};

use pierre_core::errors::protocol::ProtocolError;
use pierre_core::errors::AppError;

#[cfg(feature = "tools-data")]
use super::stored_copy::{attach_caveat, caveat};
use crate::activity_fetch::StaleReason;
use crate::protocol::types::{
    auth_required_provider, UniversalResponse, META_AUTH_REQUIRED_PROVIDER,
};
use crate::revalidation::revalidation_timeout;

#[cfg(feature = "tools-data")]
/// A provider read as it last succeeded, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct LastKnown<T> {
    /// What the provider answered.
    pub value: T,
    /// When it answered.
    pub synced_at: DateTime<Utc>,
}

#[cfg(feature = "tools-data")]
/// Keep `value` as the last-known copy under `key`.
pub(super) async fn remember<T: Serialize + Send + Sync>(cache: &Cache, key: &CacheKey, value: &T) {
    let copy = LastKnown {
        value,
        synced_at: Utc::now(),
    };
    if let Err(e) = cache.set(key, &copy, key.resource.recommended_ttl()).await {
        warn!(error = %e, "failed to keep the last-known copy of a provider read");
    }
}

#[cfg(feature = "tools-data")]
/// The last-known copy under `key`, if one is kept.
pub(super) async fn recall<T: DeserializeOwned>(
    cache: &Cache,
    key: &CacheKey,
) -> Option<LastKnown<T>> {
    cache.get::<LastKnown<T>>(key).await.ok().flatten()
}

/// How a live read bounded by the refresh budget came out.
#[derive(Debug)]
pub(super) enum LiveRead {
    /// The provider answered in time.
    Answered(UniversalResponse),
    /// It did not: why, and the failure to return when no copy stands in.
    Missed(StaleReason, UniversalResponse),
    /// The provider refused the athlete's credentials. No copy stands in:
    /// the athlete has to reconnect `provider`, and the caller raises
    /// [`ProtocolError::ProviderAuthRequired`] so the turn hands back the
    /// reconnect link (the typed-auth rule, live incident 2026-08-11).
    AuthRequired(String),
}

/// Run `read` with the refresh budget.
///
/// The read is polled in the calling task, so it keeps that task's scopes:
/// the caller's grant, and the provenance that tells the reply it served the
/// athlete's data. Only a read that outlives the budget moves to its own task,
/// where it still finishes and writes through, so a token refresh it is in
/// the middle of is never cut off. That task runs under the server's
/// [`revalidation_timeout`] from `timeouts`, so a provider that never answers
/// does not hold it forever, and a panic in it is logged rather than lost with
/// its handle.
pub(super) async fn read_within_budget<F>(timeouts: &ServerTimeouts, read: F) -> LiveRead
where
    F: Future<Output = UniversalResponse> + Send + 'static,
{
    read_within(
        RefreshConfig::default().wait_for_refresh_timeout(),
        revalidation_timeout(timeouts),
        read,
    )
    .await
}

async fn read_within<F>(budget: Duration, cap: Duration, read: F) -> LiveRead
where
    F: Future<Output = UniversalResponse> + Send + 'static,
{
    let mut read = Box::pin(read);
    match timeout(budget, &mut read).await {
        Ok(response) if response.success => LiveRead::Answered(response),
        Ok(response) => auth_required_provider(&response).map_or_else(
            || LiveRead::Missed(missed_because(&response), response),
            LiveRead::AuthRequired,
        ),
        Err(_) => {
            detach(read, cap);
            LiveRead::Missed(
                StaleReason::Timeout,
                failure(&format!(
                    "the provider did not answer within {} s; try again shortly",
                    budget.as_secs()
                )),
            )
        }
    }
}

/// Let a read the turn stopped waiting for finish in its own task, capped at
/// `cap`, and watch it so a panic or the cap is logged. It counts as what the
/// turn's reads count as: background when the turn is ([`keep_scope`]).
fn detach<F>(read: F, cap: Duration)
where
    F: Future<Output = UniversalResponse> + Send + 'static,
{
    let task = tokio::spawn(keep_scope(timeout(cap, read)));
    tokio::spawn(async move {
        match task.await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => warn!(
                timeout_secs = cap.as_secs(),
                "detached provider read did not finish within its cap"
            ),
            Err(join_error) => warn!(error = %join_error, "detached provider read failed"),
        }
    });
}

/// Why a failed read missed: [`StaleReason::ReadLimit`] when
/// [`provider_failure`] tagged it as a session-read cap's refusal, else
/// [`StaleReason::Error`].
fn missed_because(response: &UniversalResponse) -> StaleReason {
    response
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(META_READ_LIMIT_RETRY_AFTER_SECS))
        .and_then(Value::as_u64)
        .map_or(StaleReason::Error, |retry_after_secs| {
            StaleReason::ReadLimit { retry_after_secs }
        })
}

fn failure(message: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(message.to_owned()),
        metadata: None,
    }
}

/// The failure a live read reports when rendering its answer failed.
#[must_use]
pub(super) fn failed_read(error: &ProtocolError) -> UniversalResponse {
    tagged(
        failure(&error.to_string()),
        error.provider_auth_required_provider(),
    )
}

/// A provider read's failure, `context` first. A refused credential keeps its
/// provider under [`META_AUTH_REQUIRED_PROVIDER`], so [`read_within_budget`]
/// answers [`LiveRead::AuthRequired`] rather than letting a copy stand in.
///
/// The model reads the error's tool-facing message ([`AppError::tool_message`]),
/// which names the provider the athlete knows and never a backend, a service
/// or a request id; an operator reads the whole error in the log line.
///
/// A session-read cap's refusal (carnet#1661) is tagged with its wait under
/// [`META_READ_LIMIT_RETRY_AFTER_SECS`], so a copy standing in says the live
/// reads are used up; it was logged once already, on the usage target.
pub(super) fn provider_failure(context: &str, error: &AppError) -> UniversalResponse {
    let mut response = tagged(
        failure(&format!("{context}: {}", error.tool_message())),
        error.provider_auth_required_provider().as_deref(),
    );
    if session_read_cap(error).is_none() {
        warn!(context, error = %error, "provider read failed");
        return response;
    }
    debug!(context, error = %error, "provider read refused by the session-read cap");
    response.metadata.get_or_insert_with(HashMap::new).insert(
        META_READ_LIMIT_RETRY_AFTER_SECS.to_owned(),
        json!(error.retry_after_secs().unwrap_or(0)),
    );
    response
}

/// The metadata key a failed read's response carries a session-read cap's
/// wait under ([`provider_failure`]).
const META_READ_LIMIT_RETRY_AFTER_SECS: &str = "read_limit_retry_after_secs";

fn tagged(mut response: UniversalResponse, provider: Option<&str>) -> UniversalResponse {
    if let Some(provider) = provider {
        response.metadata = Some(HashMap::from([(
            META_AUTH_REQUIRED_PROVIDER.to_owned(),
            Value::String(provider.to_owned()),
        )]));
    }
    response
}

#[cfg(feature = "tools-data")]
/// Where a fallback copy is kept, and what it is to the reader.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fallback<'a> {
    /// The last-known copy's key.
    pub key: &'a CacheKey,
    /// The backend the copy came from, named in the caveat.
    pub backend: &'a str,
    /// What the copy is, in the caveat's words (`profile`, `statistics`).
    pub what: &'a str,
}

#[cfg(feature = "tools-data")]
/// `read` within the refresh budget; when it misses, the last-known copy at
/// `fallback`, rendered by `render` and carrying the caveat; with no copy, the
/// read's own failure.
///
/// # Errors
///
/// Whatever `render` returns.
pub(super) async fn live_or_last_known<T, F, R>(
    cache: &Cache,
    timeouts: &ServerTimeouts,
    fallback: Fallback<'_>,
    read: F,
    render: R,
) -> Result<UniversalResponse, ProtocolError>
where
    T: DeserializeOwned,
    F: Future<Output = UniversalResponse> + Send + 'static,
    R: FnOnce(T) -> Result<UniversalResponse, ProtocolError>,
{
    serve(
        cache,
        fallback,
        read_within_budget(timeouts, read).await,
        render,
    )
    .await
}

#[cfg(feature = "tools-data")]
async fn serve<T, R>(
    cache: &Cache,
    fallback: Fallback<'_>,
    live: LiveRead,
    render: R,
) -> Result<UniversalResponse, ProtocolError>
where
    T: DeserializeOwned,
    R: FnOnce(T) -> Result<UniversalResponse, ProtocolError>,
{
    let (reason, failed) = match live {
        LiveRead::Answered(response) => return Ok(response),
        LiveRead::Missed(reason, failed) => (reason, failed),
        LiveRead::AuthRequired(provider) => {
            return Err(ProtocolError::ProviderAuthRequired { provider })
        }
    };
    let Some(copy) = recall::<T>(cache, fallback.key).await else {
        return Ok(failed);
    };
    let mut served = render(copy.value)?;
    attach_caveat(
        &mut served,
        caveat(
            fallback.backend,
            fallback.what,
            reason,
            Some(copy.synced_at),
        ),
    );
    Ok(served)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "tools-data")]
    use pierre_cache::{CacheConfig, CacheResource};
    use pierre_core::errors::ErrorCode;
    #[cfg(feature = "tools-data")]
    use pierre_core::models::TenantId;
    use pierre_services::provider_refresh::UNREAD_PROVIDER_DIRECTIVE;
    #[cfg(feature = "tools-data")]
    use serde_json::Map;
    use tokio::sync::oneshot;
    use tokio::time::sleep;
    #[cfg(feature = "tools-data")]
    use uuid::Uuid;

    /// The detached-read cap the tests run under.
    const TEST_CAP: Duration = Duration::from_secs(2);

    #[test]
    fn a_failed_read_names_no_backend_and_keeps_the_reconnect_tag() {
        let refused = provider_failure(
            "Failed to fetch athlete profile",
            &AppError::provider_auth_required("sciotte_garmin"),
        );
        assert_eq!(
            auth_required_provider(&refused).as_deref(),
            Some("sciotte_garmin")
        );
        let timed_out = provider_failure(
            "Failed to fetch athlete profile",
            &AppError::new(
                ErrorCode::ExternalServiceUnavailable,
                "Garmin did not answer in time; try again in a moment",
            ),
        );
        let internal = provider_failure(
            "Failed to fetch athlete profile",
            &AppError::internal("sciotte athlete request (x-request-id 42) failed"),
        );
        for response in [&refused, &timed_out, &internal] {
            let said = response.error.as_deref().unwrap_or_default();
            assert!(!said.to_lowercase().contains("sciotte"), "{said}");
            assert!(!said.contains("x-request-id"), "{said}");
        }
        assert!(timed_out
            .error
            .as_deref()
            .is_some_and(|e| e.ends_with("Garmin did not answer in time; try again in a moment")));
    }

    fn ok() -> UniversalResponse {
        UniversalResponse {
            success: true,
            result: Some(json!({ "athlete": { "id": "1" } })),
            error: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn an_answer_in_time_is_served() {
        let read = read_within(Duration::from_secs(5), TEST_CAP, async { ok() }).await;
        assert!(matches!(read, LiveRead::Answered(_)));
    }

    #[tokio::test]
    async fn a_failed_read_is_missed_with_its_error() {
        let read = read_within(Duration::from_secs(5), TEST_CAP, async { failure("503") }).await;
        let LiveRead::Missed(reason, response) = read else {
            panic!("a failure must be missed");
        };
        assert_eq!(reason, StaleReason::Error);
        assert_eq!(response.error.as_deref(), Some("503"));
    }

    #[tokio::test]
    async fn a_read_over_budget_is_missed_and_still_finishes() {
        let (done_tx, done_rx) = oneshot::channel();
        let read = read_within(Duration::from_millis(20), TEST_CAP, async move {
            sleep(Duration::from_millis(100)).await;
            let _ = done_tx.send(());
            ok()
        })
        .await;
        assert!(matches!(read, LiveRead::Missed(StaleReason::Timeout, _)));
        timeout(Duration::from_secs(2), done_rx)
            .await
            .expect("the read runs on past the budget")
            .expect("the read completes");
    }

    tokio::task_local! {
        static CALLER: &'static str;
    }

    #[tokio::test]
    async fn a_refused_credential_is_auth_required_not_a_miss() {
        let refused = AppError::provider_auth_required("strava");
        let read = read_within(Duration::from_secs(5), TEST_CAP, async move {
            provider_failure("Failed to fetch athlete profile", &refused)
        })
        .await;
        let LiveRead::AuthRequired(provider) = read else {
            panic!("a refused credential must not be a miss: {read:?}");
        };
        assert_eq!(provider, "strava");
    }

    #[tokio::test]
    async fn a_plain_provider_failure_is_still_a_miss() {
        let down = AppError::external_service("strava", "503");
        let read = read_within(Duration::from_secs(5), TEST_CAP, async move {
            provider_failure("Failed to fetch athlete profile", &down)
        })
        .await;
        assert!(matches!(read, LiveRead::Missed(StaleReason::Error, _)));
    }

    /// A read the athlete's session-read cap refused misses for that reason,
    /// with its wait, so the copy standing in says live reads are used up
    /// rather than that the provider failed (carnet#1661).
    #[tokio::test]
    async fn a_capped_read_is_a_read_limit_miss_with_its_wait() {
        let mut capped = AppError::new(
            ErrorCode::ExternalRateLimited,
            "Strava has been read as often as Dravr allows for now",
        );
        capped.details = Some(Box::new(json!({
            "refused_before_send": true,
            "retry_after_secs": 420,
            "session_read_cap": "asked",
        })));
        let read = read_within(Duration::from_secs(5), TEST_CAP, async move {
            provider_failure("Failed to fetch athlete profile", &capped)
        })
        .await;
        let LiveRead::Missed(reason, failed) = read else {
            panic!("a capped read is a miss: {read:?}");
        };
        assert_eq!(
            reason,
            StaleReason::ReadLimit {
                retry_after_secs: 420
            },
            "bug: a capped read is missed for another reason, or without its wait"
        );
        assert!(
            failed
                .error
                .as_deref()
                .is_some_and(|said| said.contains("as often as Dravr allows")),
            "the model reads the cap's own message: {failed:?}"
        );
        let note = caveat("strava", "profile", reason, None);
        assert_eq!(
            note["reason"], "read_limit",
            "bug: a capped read's copy is not tagged read_limit: {note}"
        );
        assert_eq!(
            note["retry_after_secs"], 420,
            "bug: the caveat does not carry the refusal's wait: {note}"
        );
        assert!(
            note["note"]
                .as_str()
                .is_some_and(|text| text.contains("about 7 min")),
            "bug: the caveat does not say when live reads resume: {note}"
        );
    }

    #[tokio::test]
    async fn a_read_in_time_sees_the_callers_task_scope() {
        let read = CALLER
            .scope("athlete", async {
                read_within(Duration::from_secs(5), TEST_CAP, async {
                    if CALLER.try_with(|caller| *caller == "athlete") == Ok(true) {
                        ok()
                    } else {
                        failure("the read lost its caller's scope")
                    }
                })
                .await
            })
            .await;
        assert!(matches!(read, LiveRead::Answered(_)));
    }

    #[cfg(feature = "tools-data")]
    async fn cache() -> Cache {
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..Default::default()
        })
        .await
        .unwrap()
    }

    #[cfg(feature = "tools-data")]
    fn key() -> CacheKey {
        CacheKey::new(
            TenantId::from_uuid(Uuid::nil()),
            Uuid::nil(),
            "strava".to_owned(),
            CacheResource::LastKnownAthleteProfile,
        )
    }

    #[cfg(feature = "tools-data")]
    fn rendered(value: Value) -> UniversalResponse {
        let mut result = Map::new();
        result.insert("athlete".to_owned(), value);
        UniversalResponse {
            success: true,
            result: Some(Value::Object(result)),
            error: None,
            metadata: None,
        }
    }

    #[cfg(feature = "tools-data")]
    #[tokio::test]
    async fn a_missed_read_serves_the_last_known_copy_with_its_caveat() {
        let cache = cache().await;
        let key = key();
        remember(&cache, &key, &json!({ "id": "42" })).await;
        let fallback = Fallback {
            key: &key,
            backend: "strava",
            what: "profile",
        };
        let missed = LiveRead::Missed(StaleReason::Timeout, failure("late"));
        let served = serve(&cache, fallback, missed, |value: Value| Ok(rendered(value)))
            .await
            .unwrap();
        let payload = served.result.unwrap();
        assert!(served.success);
        assert_eq!(payload["athlete"]["id"], "42");
        assert_eq!(payload["provider_unavailable"]["reason"], "timeout");
        assert!(payload["provider_unavailable"]["last_synced_at"].is_string());
    }

    #[cfg(feature = "tools-data")]
    #[cfg(feature = "tools-data")]
    #[tokio::test]
    async fn a_refused_credential_never_serves_the_copy() {
        let cache = cache().await;
        let key = key();
        remember(&cache, &key, &json!({ "id": "42" })).await;
        let fallback = Fallback {
            key: &key,
            backend: "strava",
            what: "profile",
        };
        let refused = LiveRead::AuthRequired("strava".to_owned());
        let served = serve(&cache, fallback, refused, |value: Value| {
            Ok(rendered(value))
        })
        .await;
        assert!(
            matches!(&served, Err(ProtocolError::ProviderAuthRequired { provider }) if provider == "strava"),
            "{served:?}"
        );
    }

    #[tokio::test]
    async fn a_missed_read_with_no_copy_returns_its_failure() {
        let cache = cache().await;
        let key = key();
        let fallback = Fallback {
            key: &key,
            backend: "strava",
            what: "profile",
        };
        let missed = LiveRead::Missed(StaleReason::Error, failure("503"));
        let served = serve(&cache, fallback, missed, |value: Value| Ok(rendered(value)))
            .await
            .unwrap();
        assert!(!served.success);
        assert_eq!(served.error.as_deref(), Some("503"));
    }

    #[cfg(feature = "tools-data")]
    #[tokio::test]
    async fn an_answered_read_is_served_as_it_is() {
        let cache = cache().await;
        let key = key();
        remember(&cache, &key, &json!({ "id": "old" })).await;
        let fallback = Fallback {
            key: &key,
            backend: "strava",
            what: "profile",
        };
        let served = serve(
            &cache,
            fallback,
            LiveRead::Answered(ok()),
            |value: Value| Ok(rendered(value)),
        )
        .await
        .unwrap();
        assert_eq!(served.result.unwrap()["athlete"]["id"], "1");
    }

    #[test]
    fn the_caveat_says_why_and_when() {
        let at = DateTime::parse_from_rfc3339("2026-10-19T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let note = caveat("strava", "profile", StaleReason::Timeout, Some(at));
        assert_eq!(note["reason"], "timeout");
        assert_eq!(note["last_synced_at"], "2026-10-19T08:30:00+00:00");
        assert!(note["note"]
            .as_str()
            .unwrap()
            .contains("2026-10-19 08:30 UTC"));
    }

    /// A read that ran out of budget is routine: the agent answers from the
    /// copy and does not narrate the sync to the athlete.
    #[test]
    fn the_caveat_keeps_the_sync_from_the_athlete() {
        let note = caveat("strava", "statistics", StaleReason::Timeout, None);
        let text = note["note"].as_str().unwrap();
        assert!(text.contains(UNREAD_PROVIDER_DIRECTIVE), "{text}");
        assert!(!text.contains("say in one short sentence"), "{text}");
    }
}
