// ABOUTME: Single-flight for the activity pre-fetch a provider connect starts, one per (tenant, user, provider)
// ABOUTME: A reader of the same session awaits the read already in flight instead of scraping it again
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The activity pre-fetch a provider connect starts, shared with every reader
//! that arrives while it runs.
//!
//! A scrape connect fires its own reads at once: the pre-fetch that warms the
//! cache, the health backfill, and whatever screen the athlete lands on next
//! (the web onboarding asks for its coach proposal within a second). Each one
//! is a browser on the scraper service, which serves a fixed number at a time,
//! so on 2026-10-02 a COROS connect shed its own proposal read with a 503 and
//! failed a daily-summary read with a 500 (carnet#736).
//!
//! The pre-fetch registers here before it starts. A list read of the same
//! `(tenant, user, provider)` that arrives meanwhile waits for it and is
//! answered from what it read ([`PrefetchedWindow::serve`]) whenever that
//! covers the window asked for; only a window it does not cover is read live,
//! and then after the pre-fetch, never beside it. The health backfill waits on
//! the same pre-fetch before its first read ([`PrefetchTicket::wait`]).
//!
//! The registry is per process, which is where the burst happens: the connect
//! and the reads that follow it are one athlete's requests to one instance.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError};
use std::time::Duration;

use pierre_core::models::{Activity, TenantId};
use tokio::sync::watch;
use tokio::time;
use uuid::Uuid;

use crate::core::ActivityQueryParams;

/// Longest a reader waits on a pre-fetch before reading live itself.
///
/// The scraper client bounds one list read at 330 s, so a pre-fetch still
/// running past this is wedged, and the reader stops waiting on it.
pub const PREFETCH_WAIT_CAP: Duration = Duration::from_mins(6);

/// Whose pre-fetch: the tenant, the athlete and the provider backend the
/// connection is stored under (`sciotte_coros`, not `coros`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrefetchKey {
    tenant_id: TenantId,
    user_id: Uuid,
    provider: String,
}

impl PrefetchKey {
    /// The key of `user_id`'s `provider` connection in `tenant_id`.
    #[must_use]
    pub fn new(tenant_id: TenantId, user_id: Uuid, provider: impl Into<String>) -> Self {
        Self {
            tenant_id,
            user_id,
            provider: provider.into(),
        }
    }
}

/// What a finished pre-fetch read: the provider's newest activities, as many
/// as it asked for at most.
#[derive(Debug, Clone)]
pub struct PrefetchedWindow {
    activities: Vec<Activity>,
    limit: usize,
    head_complete: bool,
}

impl PrefetchedWindow {
    /// The `activities` a read of the newest `limit` answered, and whether
    /// its capture reached the list head.
    #[must_use]
    pub const fn new(activities: Vec<Activity>, limit: usize, head_complete: bool) -> Self {
        Self {
            activities,
            limit,
            head_complete,
        }
    }

    /// Whether the pre-fetch's capture reached the provider's list head.
    #[must_use]
    pub const fn head_complete(&self) -> bool {
        self.head_complete
    }

    /// The answer to a read of `params`, out of this window, or `None` when
    /// the window cannot vouch for it.
    ///
    /// The pre-fetch read the newest activities without a lower bound, so it
    /// holds every activity from the list head down to its oldest one. A read
    /// of the first page is therefore answered whole when the pre-fetch
    /// reached the end of the account's history (it got fewer than it asked
    /// for), reached below the read's `after`, or holds at least the read's
    /// `limit` inside the window. A later page, or a deeper window than the
    /// pre-fetch reached, is the read's own to make.
    #[must_use]
    pub fn serve(&self, params: &ActivityQueryParams) -> Option<Vec<Activity>> {
        if params.offset.unwrap_or(0) > 0 {
            return None;
        }
        let history_exhausted = self.activities.len() < self.limit;
        let reaches_after = params
            .after
            .zip(
                self.activities
                    .iter()
                    .map(|a| a.start_date().timestamp())
                    .min(),
            )
            .is_some_and(|(after, oldest)| oldest <= after);
        let mut window: Vec<Activity> = self
            .activities
            .iter()
            .filter(|a| {
                let start = a.start_date().timestamp();
                params.after.is_none_or(|after| start > after)
                    && params.before.is_none_or(|before| start < before)
            })
            .cloned()
            .collect();
        let fills_limit = params.limit.is_some_and(|limit| window.len() >= limit);
        if !(history_exhausted || reaches_after || fills_limit) {
            return None;
        }
        window.sort_by_key(|a| Reverse(a.start_date()));
        if let Some(limit) = params.limit {
            window.truncate(limit);
        }
        Some(window)
    }
}

/// A pre-fetch's outcome as its waiters see it: `None` while it runs, then
/// the window it read, or `None` inside when it read nothing usable.
type Outcome = Option<Option<Arc<PrefetchedWindow>>>;

type InFlight = Arc<StdMutex<HashMap<PrefetchKey, watch::Receiver<Outcome>>>>;

/// The pre-fetches running in this process, one per [`PrefetchKey`].
///
/// Production code shares one registry through [`Self::global`]; a unit test
/// builds its own with [`Self::new`].
pub struct ConnectPrefetches {
    in_flight: InFlight,
}

impl ConnectPrefetches {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            in_flight: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// The registry every connect and every reader of this process shares.
    #[must_use]
    pub fn global() -> &'static Self {
        static GLOBAL: LazyLock<ConnectPrefetches> = LazyLock::new(ConnectPrefetches::new);
        &GLOBAL
    }

    /// Register the pre-fetch of `key`, before it starts.
    ///
    /// `None` when one is already in flight for the key: the caller then
    /// starts no second one, and its readers wait on the first.
    pub fn begin(&self, key: PrefetchKey) -> Option<PrefetchTicket> {
        // A holder that panicked leaves the map consistent; refusing every
        // later pre-fetch would be worse than recovering it.
        let mut in_flight = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if in_flight.contains_key(&key) {
            return None;
        }
        let (sender, receiver) = watch::channel(None);
        in_flight.insert(key.clone(), receiver);
        Some(PrefetchTicket {
            in_flight: Arc::clone(&self.in_flight),
            key,
            sender,
        })
    }

    /// The pre-fetch of `key` still running, to wait on; `None` when none is.
    #[must_use]
    pub fn in_flight(&self, key: &PrefetchKey) -> Option<PrefetchWait> {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
            .map(|receiver| PrefetchWait { receiver })
    }
}

impl Default for ConnectPrefetches {
    fn default() -> Self {
        Self::new()
    }
}

/// A running pre-fetch's hold on its key, from before its read starts until
/// it has published what the read came to.
///
/// Dropped without [`Self::finish`] — the pre-fetch returned early or
/// panicked — it frees the key and its waiters read live.
pub struct PrefetchTicket {
    in_flight: InFlight,
    key: PrefetchKey,
    sender: watch::Sender<Outcome>,
}

impl PrefetchTicket {
    /// A handle on this pre-fetch's outcome, for work that must start only
    /// after it.
    #[must_use]
    pub fn wait(&self) -> PrefetchWait {
        PrefetchWait {
            receiver: self.sender.subscribe(),
        }
    }

    /// Publish what the pre-fetch read (`None` when it read nothing usable)
    /// to every waiter, and free the key.
    pub fn finish(self, window: Option<PrefetchedWindow>) {
        self.sender.send_replace(Some(window.map(Arc::new)));
    }
}

impl Drop for PrefetchTicket {
    fn drop(&mut self) {
        // Only this ticket's holder ever removes the key, and `begin` inserts
        // none while it is held, so the entry under the key is this one.
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.key);
    }
}

/// A wait on one pre-fetch's outcome.
pub struct PrefetchWait {
    receiver: watch::Receiver<Outcome>,
}

impl PrefetchWait {
    /// What the pre-fetch read, once it has finished: `None` when it read
    /// nothing usable, ended without publishing, or ran past
    /// [`PREFETCH_WAIT_CAP`].
    pub async fn finished(mut self) -> Option<Arc<PrefetchedWindow>> {
        let outcome =
            time::timeout(PREFETCH_WAIT_CAP, self.receiver.wait_for(Option::is_some)).await;
        match outcome {
            Ok(Ok(published)) => published.clone().flatten(),
            Ok(Err(_)) | Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Duration as ChronoDuration, Utc};
    use pierre_core::models::{ActivityBuilder, SportType};

    fn ride(id: &str, start: DateTime<Utc>) -> Activity {
        ActivityBuilder::new(id, "ride", SportType::Ride, start, 3600, "sciotte_coros").build()
    }

    fn days_ago(days: i64) -> DateTime<Utc> {
        Utc::now() - ChronoDuration::days(days)
    }

    fn first_page(limit: usize, after: Option<i64>) -> ActivityQueryParams {
        ActivityQueryParams {
            limit: Some(limit),
            offset: None,
            before: None,
            after,
        }
    }

    fn key() -> PrefetchKey {
        PrefetchKey::new(
            TenantId::from_uuid(Uuid::new_v4()),
            Uuid::new_v4(),
            "sciotte_coros",
        )
    }

    #[test]
    fn a_window_that_reached_the_end_of_history_answers_any_first_page() {
        let window = PrefetchedWindow::new(
            vec![ride("a", days_ago(1)), ride("b", days_ago(200))],
            30,
            true,
        );
        let served = window
            .serve(&first_page(50, Some(days_ago(90).timestamp())))
            .unwrap();
        assert_eq!(
            served.len(),
            1,
            "the 200-day-old ride is outside the window"
        );
        assert_eq!(served[0].id(), "a");
    }

    #[test]
    fn a_full_window_answers_only_what_it_reaches() {
        let full: Vec<Activity> = (0..3)
            .map(|d| ride(&format!("r{d}"), days_ago(d)))
            .collect();
        let window = PrefetchedWindow::new(full, 3, true);
        // Its oldest ride is two days old: a 90-day window may hold more.
        assert!(window
            .serve(&first_page(50, Some(days_ago(90).timestamp())))
            .is_none());
        // The newest two are wholly inside it.
        assert_eq!(window.serve(&first_page(2, None)).unwrap().len(), 2);
        // A later page is the reader's own.
        let mut later = first_page(2, None);
        later.offset = Some(2);
        assert!(window.serve(&later).is_none());
    }

    #[tokio::test]
    async fn a_reader_waits_for_the_prefetch_and_gets_what_it_read() {
        let registry = ConnectPrefetches::new();
        let ticket = registry.begin(key()).unwrap();
        assert!(
            registry.begin(ticket.key.clone()).is_none(),
            "one pre-fetch per key"
        );
        let wait = registry.in_flight(&ticket.key).unwrap();
        let key = ticket.key.clone();
        tokio::spawn(async move {
            ticket.finish(Some(PrefetchedWindow::new(
                vec![ride("a", days_ago(1))],
                30,
                true,
            )));
        });
        let window = wait.finished().await.unwrap();
        assert_eq!(window.serve(&first_page(30, None)).unwrap().len(), 1);
        assert!(
            registry.in_flight(&key).is_none(),
            "the key is freed once it finished"
        );
    }

    #[tokio::test]
    async fn a_prefetch_dropped_unfinished_releases_its_waiters() {
        let registry = ConnectPrefetches::new();
        let ticket = registry.begin(key()).unwrap();
        let wait = ticket.wait();
        let key = ticket.key.clone();
        drop(ticket);
        assert!(wait.finished().await.is_none());
        assert!(registry.in_flight(&key).is_none());
        assert!(
            registry.begin(key).is_some(),
            "the key is free for the next connect"
        );
    }
}
