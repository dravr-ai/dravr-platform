// ABOUTME: The handle a credential carries to be admitted against its holder's request windows
// ABOUTME: RequestBudget: an app's, a personal API key's or a signed-in session's, and the free admit helper

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The handle a credential carries to be admitted against the request
//! windows its holder is counted in ([`ProviderRateLimiter`]).

use std::fmt;
use std::sync::Arc;

use pierre_core::errors::AppResult;
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-sciotte",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-wahoo",
    feature = "provider-whoop",
))]
use tokio::sync::RwLock;
use tracing::error;
use uuid::Uuid;

use super::session_read::{self, SESSION_READ_BACKENDS};
use super::{
    budget_refusal, is_background, Holder, ProviderRateLimiter, RateLimitStatus, SESSION_BACKENDS,
};
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-sciotte",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-wahoo",
    feature = "provider-whoop",
))]
use crate::core::OAuth2Credentials;

/// The budget a credential's provider requests are admitted against.
///
/// The shared windows of the OAuth app that signs them, those its provider
/// keeps for a personal API key, the walk windows of the app a signed-in
/// session's detail walk is counted in, an athlete's context-read windows,
/// or the session-read windows every read of an athlete's session spends.
#[derive(Clone)]
pub struct RequestBudget {
    limiter: Arc<ProviderRateLimiter>,
    holder: Holder,
}

impl RequestBudget {
    /// The windows of `app` (the signing client's id) in `limiter`, with the
    /// app's own `daily_limit` when it registered one.
    #[must_use]
    pub const fn new(
        limiter: Arc<ProviderRateLimiter>,
        app: String,
        daily_limit: Option<u32>,
    ) -> Self {
        Self {
            limiter,
            holder: Holder::App { app, daily_limit },
        }
    }

    /// The windows the provider keeps for one personal API key, counted for
    /// `account`, the provider's id for the account the key belongs to. No
    /// app signs a key, so no app's windows apply.
    #[must_use]
    pub const fn for_api_key(limiter: Arc<ProviderRateLimiter>, account: String) -> Self {
        Self {
            limiter,
            holder: Holder::ApiKey { account },
        }
    }

    /// The budget the detail walk of `backend`'s signed-in session is
    /// counted in, when the limiter counts that session: the daily walk
    /// windows of the Strava app `signing` carries, the athlete's own grant,
    /// so a session step and a grant step draw from one pool, or of the
    /// server's own app `server_app` when the athlete has no grant. `None`
    /// for a backend whose sessions it does not count.
    #[must_use]
    pub(crate) fn for_session(
        limiter: &Arc<ProviderRateLimiter>,
        backend: &str,
        signing: Option<&Self>,
        server_app: &str,
    ) -> Option<Self> {
        if !SESSION_BACKENDS.contains(&backend) {
            return None;
        }
        let (app, daily_limit) = match signing.map(|budget| &budget.holder) {
            Some(Holder::App { app, daily_limit } | Holder::Session { app, daily_limit }) => {
                (app.clone(), *daily_limit)
            }
            Some(
                Holder::ApiKey { .. } | Holder::SessionContext { .. } | Holder::SessionRead { .. },
            )
            | None => (server_app.to_owned(), None),
        };
        Some(Self {
            limiter: Arc::clone(limiter),
            holder: Holder::Session { app, daily_limit },
        })
    }

    /// The budget the live provider context reads (carnet#1622) of
    /// `backend`'s signed-in session are counted in for `user_id`: that
    /// athlete's own windows, apart from the walks' and any app's
    /// ([`super::context_read`]). `None` for a backend whose sessions the
    /// limiter does not count.
    #[must_use]
    pub(crate) fn for_session_context(
        limiter: &Arc<ProviderRateLimiter>,
        backend: &str,
        user_id: Uuid,
    ) -> Option<Self> {
        SESSION_BACKENDS.contains(&backend).then(|| Self {
            limiter: Arc::clone(limiter),
            holder: Holder::SessionContext {
                backend: backend.to_owned(),
                user_id,
            },
        })
    }

    /// The budget every read of `user_id`'s signed-in session at `backend`
    /// is counted in: that athlete's own session-read windows at that
    /// backend, sized by their tier ([`super::session_read`]). `None` for a
    /// backend that is not one of the [`SESSION_READ_BACKENDS`].
    #[must_use]
    pub fn for_session_read(
        limiter: &Arc<ProviderRateLimiter>,
        backend: &str,
        user_id: Uuid,
    ) -> Option<Self> {
        SESSION_READ_BACKENDS.contains(&backend).then(|| Self {
            limiter: Arc::clone(limiter),
            holder: Holder::SessionRead {
                backend: backend.to_owned(),
                user_id,
            },
        })
    }

    /// Admit one request to `provider`, or refuse it with
    /// [`ErrorCode::ExternalRateLimited`](pierre_core::errors::ErrorCode::ExternalRateLimited)
    /// and the wait until the spent window resets.
    ///
    /// A window that cannot be counted refuses work nobody is waiting on and
    /// lets a request an athlete is waiting on through, logged as an error:
    /// the budget protects the provider grant, and an unreachable database is
    /// an outage of its own, paged by that log.
    ///
    /// # Errors
    /// Returns the refusal, or for background work the counting error.
    pub async fn admit(&self, provider: &str) -> AppResult<()> {
        match self.limiter.acquire_for(provider, &self.holder).await {
            Ok(RateLimitStatus::Allowed) => Ok(()),
            Ok(RateLimitStatus::Exceeded { retry_after }) => match &self.holder {
                Holder::SessionRead { backend, user_id } => {
                    let disabled = self.limiter.session_reads_disabled(*user_id).await;
                    Err(session_read::refusal(backend, retry_after, disabled))
                }
                _ => Err(budget_refusal(provider, retry_after)),
            },
            Err(e) if is_background() => Err(e),
            Err(e) => {
                error!(provider, app = self.holder.app(), error = %e, "provider request budget could not be counted; the request is sent uncounted");
                Ok(())
            }
        }
    }

    /// The calls a day one athlete's detail walk at `provider` may make
    /// under this budget's holder, as the limiter sizes the walk's window:
    /// [`DETAIL_WALK_DAILY_CALLS`](super::DETAIL_WALK_DAILY_CALLS), held to
    /// the app's walk window, which is half the background share of the
    /// holder's actual day (`0` for a day under 8). A walk's ETA is paced by it, and a walk with no room at all
    /// is never recorded.
    ///
    /// `None` when `provider` has no walk windows, or the holder no daily
    /// window: its walk is held to its background share alone.
    pub async fn detail_walk_daily_calls(&self, provider: &str) -> Option<u32> {
        self.limiter
            .detail_walk_daily_calls(provider, &self.holder)
            .await
    }
}

impl fmt::Debug for RequestBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBudget")
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

/// The budget the credentials in a provider's slot carry, cloned out so the
/// lock is released before the request is admitted.
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-sciotte",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-wahoo",
    feature = "provider-whoop",
))]
pub(crate) async fn carried_by(
    credentials: &RwLock<Option<OAuth2Credentials>>,
) -> Option<RequestBudget> {
    credentials
        .read()
        .await
        .as_ref()
        .and_then(|credentials| credentials.request_budget.clone())
}

/// Admit one request to `provider` against `budget`, when the credential
/// carries one.
///
/// # Errors
/// Returns [`RequestBudget::admit`]'s refusal.
pub async fn admit(budget: Option<&RequestBudget>, provider: &str) -> AppResult<()> {
    match budget {
        Some(budget) => budget.admit(provider).await,
        None => Ok(()),
    }
}
