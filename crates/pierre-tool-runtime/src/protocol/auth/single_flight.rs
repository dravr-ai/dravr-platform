// ABOUTME: Single-flight: concurrent callers of one keyed operation share the one run in flight
// ABOUTME: The run is a detached task, so a caller that goes away abandons its wait and never the run
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::watch;
use tracing::Instrument;

/// The runs in flight, by key: where a caller that finds one waits for its
/// result.
type Flights<K, V> = HashMap<K, watch::Receiver<Option<V>>>;

/// One run at a time per key, shared by everyone who asks while it runs.
///
/// The first caller for a key launches the run, as a task of its own, and
/// waits for its result like everyone else. A caller that arrives while that
/// run is in flight launches nothing and gets a clone of the same result, an
/// error as much as a success. Keys are independent, so a slow run delays
/// only its own key.
///
/// The run belongs to no caller. One that is dropped mid-wait (a client that
/// disconnected, an outer timeout) abandons its wait and nothing else: the
/// run goes on to its end, and whoever still waits, or asks while it runs,
/// gets its result. That matters when the work cannot be taken back halfway,
/// as a refresh that has spent a rotating refresh token cannot.
///
/// The map holds a key only while its run is in flight, so it is bounded by
/// the number of runs in progress and is empty at rest. Its lock is a plain
/// mutex held for a lookup, an insert or a removal, and never across an
/// `await`: a waiter waits on the run's channel, not on the lock.
pub(super) struct SingleFlight<K, V> {
    /// Shared with each run's [`Landing`], which frees its key from the
    /// spawned task after the caller that launched it may be gone.
    flights: Arc<Mutex<Flights<K, V>>>,
}

/// A run that ended without a result: its work panicked, or the runtime it
/// ran on shut down under it. Whatever the work had done by then stands, and
/// nothing is known about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Lost;

/// The run's hold on its key and on the channel its waiters watch.
///
/// The key is freed before anyone waiting wakes, however the run ends, so a
/// caller that asks once the run is over starts a run of its own instead of
/// joining one that is finished. A run that finishes frees it in
/// [`Self::land`]; one that panicked, or was torn down with its runtime, in
/// `drop`, which runs before the fields are dropped and the channel closes.
struct Landing<K: Eq + Hash, V> {
    /// The [`SingleFlight`]'s map, shared so the spawned run can free its key
    /// without borrowing from the caller that launched it.
    flights: Arc<Mutex<Flights<K, V>>>,
    /// The key this run holds.
    key: K,
    /// Where the run's result reaches everyone waiting on it.
    publish: watch::Sender<Option<V>>,
    /// Whether [`Self::land`] already freed the key: `drop` must not free it
    /// again, since by then it may be the key of the next run.
    freed: bool,
}

impl<K: Eq + Hash, V> Landing<K, V> {
    /// Free the key, then hand `value` to everyone waiting on the run.
    fn land(mut self, value: V) {
        lock(&self.flights).remove(&self.key);
        self.freed = true;
        self.publish.send_replace(Some(value));
    }
}

impl<K: Eq + Hash, V> Drop for Landing<K, V> {
    fn drop(&mut self) {
        if !self.freed {
            lock(&self.flights).remove(&self.key);
        }
    }
}

/// The map, whatever a panic left of the lock: every critical section is one
/// map operation, so a poisoned lock still guards a consistent map.
fn lock<K, V>(flights: &Mutex<Flights<K, V>>) -> MutexGuard<'_, Flights<K, V>> {
    flights.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<K, V> SingleFlight<K, V> {
    /// No run in flight.
    pub(super) fn new() -> Self {
        Self {
            flights: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<K, V> SingleFlight<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// Wait for the run of `key` in flight, launching it from `work` when
    /// there is none, and return what it returns.
    ///
    /// `work` is called by the caller that launches the run and by no other,
    /// and the future it returns owns everything it needs: it is spawned, and
    /// outlives this call when the caller is dropped.
    ///
    /// # Errors
    /// Returns [`Lost`] when the run ended without a result. Every caller
    /// waiting on it gets the same, and none is left waiting; the key is free
    /// again for the next call.
    pub(super) async fn run<F, Fut>(&self, key: K, work: F) -> Result<V, Lost>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V> + Send + 'static,
    {
        let mut flight = self.join_or_launch(key, work);
        flight
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|value| value.clone())
            .ok_or(Lost)
    }

    /// The channel of the run in flight for `key`: the one already there, or
    /// one this call registers and launches.
    ///
    /// Not `async`: between registering the key and spawning its run there is
    /// no `await` at which a dropped caller could leave a key with no run
    /// behind it.
    fn join_or_launch<F, Fut>(&self, key: K, work: F) -> watch::Receiver<Option<V>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = V> + Send + 'static,
    {
        let (landing, flight) = {
            let mut flights = lock(&self.flights);
            if let Some(flight) = flights.get(&key) {
                return flight.clone();
            }
            let (publish, flight) = watch::channel(None);
            flights.insert(key.clone(), flight.clone());
            let landing = Landing {
                flights: Arc::clone(&self.flights),
                key,
                publish,
                freed: false,
            };
            (landing, flight)
        };
        // `landing` already holds the key, so a `work` that panics while
        // building its future frees it too.
        let run = work();
        // The launching caller's span, so what the run logs still carries the
        // context of the request that started it (the turn, the tool call).
        tokio::spawn(async move { landing.land(run.await) }.in_current_span());
        flight
    }
}

#[cfg(test)]
mod tests {
    use super::{lock, Lost, SingleFlight};
    use std::future::Ready;
    use std::hash::Hash;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio::task::yield_now;
    use tokio::time::timeout;

    /// Long enough that only a hung waiter reaches it.
    const HUNG: Duration = Duration::from_secs(5);

    impl<K: Eq + Hash, V> SingleFlight<K, V> {
        fn in_flight(&self) -> usize {
            lock(&self.flights).len()
        }
    }

    /// Wait until `flights` holds `count` keys: the runs under test have
    /// registered, or landed.
    async fn until_in_flight<V: Send + Sync>(
        flights: &SingleFlight<&'static str, V>,
        count: usize,
    ) {
        timeout(HUNG, async {
            while flights.in_flight() != count {
                yield_now().await;
            }
        })
        .await
        .expect("the runs register");
    }

    /// Let every task spawned so far reach its wait.
    async fn settle() {
        for _ in 0..20 {
            yield_now().await;
        }
    }

    #[tokio::test]
    async fn followers_get_the_leaders_result_from_one_run() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());

        let callers: Vec<_> = (0..5)
            .map(|_| {
                let (flights, runs, release) = (
                    Arc::clone(&flights),
                    Arc::clone(&runs),
                    Arc::clone(&release),
                );
                tokio::spawn(async move {
                    flights
                        .run("athlete", || async move {
                            runs.fetch_add(1, Ordering::SeqCst);
                            release.notified().await;
                            7
                        })
                        .await
                })
            })
            .collect();
        until_in_flight(&flights, 1).await;
        // Every caller has reached the flight before the run is released.
        settle().await;
        release.notify_one();

        for caller in callers {
            assert_eq!(timeout(HUNG, caller).await.unwrap().unwrap(), Ok(7));
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1, "one run served them all");
        assert_eq!(flights.in_flight(), 0, "a finished run leaves no key");
    }

    #[tokio::test]
    async fn an_unrelated_key_is_not_held_up() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let release = Arc::new(Notify::new());
        let slow = {
            let (flights, release) = (Arc::clone(&flights), Arc::clone(&release));
            tokio::spawn(async move {
                flights
                    .run("slow", || async move {
                        release.notified().await;
                        1
                    })
                    .await
            })
        };
        until_in_flight(&flights, 1).await;

        let other = timeout(HUNG, flights.run("other", || async { 2 }))
            .await
            .expect("another key runs while the first is in flight");
        assert_eq!(other, Ok(2));

        release.notify_one();
        assert_eq!(timeout(HUNG, slow).await.unwrap().unwrap(), Ok(1));
        until_in_flight(&flights, 0).await;
    }

    /// The caller that launched a run is dropped while it is in flight. The
    /// run is not its to cancel: it finishes, once, and the caller still
    /// waiting on it gets its result instead of running the work again.
    #[tokio::test]
    async fn a_cancelled_leader_leaves_its_run_to_finish_for_its_follower() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let leader = {
            let (flights, runs, release) = (
                Arc::clone(&flights),
                Arc::clone(&runs),
                Arc::clone(&release),
            );
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        release.notified().await;
                        1
                    })
                    .await
            })
        };
        until_in_flight(&flights, 1).await;
        let follower = {
            let (flights, runs) = (Arc::clone(&flights), Arc::clone(&runs));
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        2
                    })
                    .await
            })
        };
        settle().await;

        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        settle().await;
        assert_eq!(
            flights.in_flight(),
            1,
            "the run outlives the caller that launched it"
        );
        assert!(
            !follower.is_finished(),
            "and its follower still waits on it"
        );

        release.notify_one();
        assert_eq!(
            timeout(HUNG, follower)
                .await
                .expect("the follower is not left waiting on a cancelled leader")
                .unwrap(),
            Ok(1),
            "the follower gets the result of the run the leader launched"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the work ran once");
        until_in_flight(&flights, 0).await;
    }

    /// A run with nobody left waiting on it still finishes: every caller was
    /// dropped, and the work is done all the same.
    #[tokio::test]
    async fn a_run_every_caller_abandoned_still_finishes() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let finished = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let only_caller = {
            let (flights, finished, release) = (
                Arc::clone(&flights),
                Arc::clone(&finished),
                Arc::clone(&release),
            );
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        release.notified().await;
                        finished.fetch_add(1, Ordering::SeqCst);
                        1
                    })
                    .await
            })
        };
        until_in_flight(&flights, 1).await;

        only_caller.abort();
        assert!(only_caller.await.unwrap_err().is_cancelled());
        release.notify_one();

        until_in_flight(&flights, 0).await;
        assert_eq!(finished.load(Ordering::SeqCst), 1, "the work completed");
    }

    /// Work that panics takes down its run and nobody else: the caller that
    /// launched it and its follower both get [`Lost`], none hangs, and the
    /// key runs again.
    #[tokio::test]
    async fn a_panicked_run_frees_the_key_and_fails_every_waiter() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let runs = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Notify::new());
        let leader = {
            let (flights, release) = (Arc::clone(&flights), Arc::clone(&release));
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        release.notified().await;
                        panic!("the run's work panics");
                    })
                    .await
            })
        };
        until_in_flight(&flights, 1).await;
        let follower = {
            let (flights, runs) = (Arc::clone(&flights), Arc::clone(&runs));
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        runs.fetch_add(1, Ordering::SeqCst);
                        2
                    })
                    .await
            })
        };
        settle().await;

        release.notify_one();
        assert_eq!(
            timeout(HUNG, leader)
                .await
                .expect("the launching caller is not left waiting on a panicked run")
                .expect("the panic stays in the run's task"),
            Err(Lost)
        );
        assert_eq!(
            timeout(HUNG, follower)
                .await
                .expect("the follower is not left waiting on a panicked run")
                .unwrap(),
            Err(Lost)
        );
        assert_eq!(
            runs.load(Ordering::SeqCst),
            0,
            "the follower did not run the work over whatever the panic left"
        );
        assert_eq!(flights.in_flight(), 0, "no key is left stuck");
        assert_eq!(
            timeout(HUNG, flights.run("athlete", || async { 3 }))
                .await
                .expect("the key runs again after a panic"),
            Ok(3)
        );
    }

    /// A `work` that panics before it returns its future has registered the
    /// key already; the key is freed all the same.
    #[tokio::test]
    async fn work_that_panics_before_its_future_frees_the_key() {
        let flights = Arc::new(SingleFlight::<&'static str, u32>::new());
        let caller = {
            let flights = Arc::clone(&flights);
            tokio::spawn(async move {
                flights
                    .run("athlete", || -> Ready<u32> {
                        panic!("building the work panics")
                    })
                    .await
            })
        };
        assert!(caller.await.unwrap_err().is_panic());

        assert_eq!(flights.in_flight(), 0, "no key is left stuck");
        assert_eq!(
            timeout(HUNG, flights.run("athlete", || async { 3 }))
                .await
                .expect("the key runs again"),
            Ok(3)
        );
    }

    #[tokio::test]
    async fn a_failed_leader_hands_its_error_to_its_followers() {
        let flights = Arc::new(SingleFlight::<&'static str, Result<u32, String>>::new());
        let release = Arc::new(Notify::new());
        let leader = {
            let (flights, release) = (Arc::clone(&flights), Arc::clone(&release));
            tokio::spawn(async move {
                flights
                    .run("athlete", || async move {
                        release.notified().await;
                        Err("refused".to_owned())
                    })
                    .await
            })
        };
        until_in_flight(&flights, 1).await;
        let follower = {
            let flights = Arc::clone(&flights);
            tokio::spawn(async move { flights.run("athlete", || async { Ok(9) }).await })
        };
        settle().await;
        release.notify_one();

        assert_eq!(leader.await.unwrap(), Ok(Err("refused".to_owned())));
        assert_eq!(
            timeout(HUNG, follower).await.unwrap().unwrap(),
            Ok(Err("refused".to_owned())),
            "the follower gets the leader's failure, and runs nothing"
        );
        until_in_flight(&flights, 0).await;
    }
}
