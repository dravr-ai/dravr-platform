// ABOUTME: Per-callsite log throttle — a line repeating past its budget in a window is dropped and counted
// ABOUTME: A reporter thread writes one summary per throttled callsite per window with the dropped count
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Callsite throttle
//!
//! During an incident one `warn!` in a retry loop can write thousands of
//! identical lines a second: Cloud Logging bills each one and the line that
//! explains the incident scrolls away under them. [`CallsiteThrottle`] is a
//! per-layer filter on the formatter that lets each callsite write `burst`
//! events per window, drops the rest, and counts them. A reporter thread then
//! writes one summary per throttled callsite per window, so nothing vanishes
//! silently: the summary names the callsite and how many lines it dropped.
//!
//! Log-based metrics in `infra/` count lines, so some events are never
//! throttled:
//!
//! - **ERROR**: `cloud_run_job_failures` counts every ERROR line a job writes.
//! - **an `event` field**: the structured events metrics select on
//!   (`chat.question_asked`, `key_material.*`, `database.key_store_unavailable`).
//! - **the `notify` target**: business events routed to Slack.
//!
//! A metric that counts a WARN or INFO message still fires during a flood,
//! since the first `burst` lines of each window pass; it only stops counting
//! past that. The budget is far above what any of those callsites writes in a
//! window.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use dravr_tronc::NOTIFY_TARGET;
use tracing::callsite::Identifier;
use tracing::{Event, Level, Metadata, Subscriber};
use tracing_subscriber::layer::{Context, Filter};

/// Target of the summary lines the reporter writes. Never throttled itself.
pub(crate) const THROTTLE_TARGET: &str = "pierre_logging::throttle";

/// Length of one throttle window.
pub(crate) const THROTTLE_WINDOW: Duration = Duration::from_secs(10);

/// Events one callsite may write per window before the rest are dropped.
pub(crate) const DEFAULT_THROTTLE_BURST: u64 = 100;

/// One callsite's counts for the current window.
struct CallsiteCount {
    metadata: &'static Metadata<'static>,
    /// Index of the window `passed` counts for.
    window: AtomicU64,
    /// Events let through in `window`.
    passed: AtomicU64,
    /// Events dropped since the reporter last ran.
    suppressed: AtomicU64,
}

/// Counts shared by the filter and the reporter.
struct ThrottleState {
    burst: u64,
    window: Duration,
    started: Instant,
    counts: RwLock<HashMap<Identifier, Arc<CallsiteCount>>>,
}

impl ThrottleState {
    fn window_index(&self) -> u64 {
        let elapsed = self.started.elapsed().as_millis();
        let width = self.window.as_millis().max(1);
        u64::try_from(elapsed / width).unwrap_or(u64::MAX)
    }

    fn count_for(&self, metadata: &'static Metadata<'static>) -> Arc<CallsiteCount> {
        let id = metadata.callsite();
        if let Some(count) = self
            .counts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&id)
        {
            return Arc::clone(count);
        }
        let mut counts = self.counts.write().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(counts.entry(id).or_insert_with(|| {
            Arc::new(CallsiteCount {
                metadata,
                window: AtomicU64::new(self.window_index()),
                passed: AtomicU64::new(0),
                suppressed: AtomicU64::new(0),
            })
        }))
    }

    /// Whether this event of `metadata`'s callsite fits the current window.
    fn admit(&self, metadata: &'static Metadata<'static>) -> bool {
        let count = self.count_for(metadata);
        let now = self.window_index();
        let seen = count.window.load(Ordering::Acquire);
        if seen != now
            && count
                .window
                .compare_exchange(seen, now, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            count.passed.store(0, Ordering::Release);
        }
        if count.passed.fetch_add(1, Ordering::AcqRel) < self.burst {
            return true;
        }
        count.suppressed.fetch_add(1, Ordering::AcqRel);
        false
    }

    /// Takes every callsite's dropped count, leaving zero behind.
    fn drain(&self) -> Vec<(&'static Metadata<'static>, u64)> {
        self.counts
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter_map(|count| {
                let dropped = count.suppressed.swap(0, Ordering::AcqRel);
                (dropped > 0).then_some((count.metadata, dropped))
            })
            .collect()
    }
}

/// Per-layer filter that drops a callsite's events past its per-window budget.
///
/// Clones share their counts, so the filter and its [`ThrottleReporter`] see
/// the same windows.
#[derive(Clone)]
pub(crate) struct CallsiteThrottle {
    state: Arc<ThrottleState>,
}

impl CallsiteThrottle {
    /// A throttle letting `burst` events per callsite through each `window`;
    /// a `burst` of `0` lets everything through.
    #[must_use]
    pub(crate) fn new(burst: u64, window: Duration) -> Self {
        Self {
            state: Arc::new(ThrottleState {
                burst,
                window,
                started: Instant::now(),
                counts: RwLock::new(HashMap::new()),
            }),
        }
    }

    /// The reporter that writes this throttle's summaries.
    #[must_use]
    pub(crate) fn reporter(&self) -> ThrottleReporter {
        ThrottleReporter {
            state: Arc::clone(&self.state),
        }
    }
}

/// Whether an event is one a log-based metric may count, and so never dropped.
fn is_exempt(metadata: &Metadata<'_>) -> bool {
    *metadata.level() == Level::ERROR
        || metadata.target() == NOTIFY_TARGET
        || metadata.target() == THROTTLE_TARGET
        || metadata.fields().field("event").is_some()
}

impl<S: Subscriber> Filter<S> for CallsiteThrottle {
    fn enabled(&self, _metadata: &Metadata<'_>, _ctx: &Context<'_, S>) -> bool {
        // Counting happens per event; a callsite is never disabled outright.
        true
    }

    fn event_enabled(&self, event: &Event<'_>, _ctx: &Context<'_, S>) -> bool {
        let metadata = event.metadata();
        self.state.burst == 0 || is_exempt(metadata) || self.state.admit(metadata)
    }
}

/// Writes one summary per throttled callsite per window.
///
/// An event written from inside a filter is dropped (the dispatcher is busy
/// with the event being filtered), so the summaries come from outside it.
pub(crate) struct ThrottleReporter {
    state: Arc<ThrottleState>,
}

impl ThrottleReporter {
    /// Writes a summary for every callsite that dropped lines since the last
    /// call, and returns how many callsites it reported.
    pub(crate) fn report(&self) -> usize {
        let drained = self.state.drain();
        for (metadata, dropped) in &drained {
            tracing::warn!(
                target: THROTTLE_TARGET,
                callsite = metadata.name(),
                callsite_target = metadata.target(),
                level = %metadata.level(),
                dropped = *dropped,
                window_secs = self.state.window.as_secs(),
                "repeated log line throttled"
            );
        }
        drained.len()
    }

    /// Reports once per window on a background thread for the life of the
    /// process.
    ///
    /// # Errors
    ///
    /// Returns the OS error when the thread cannot be spawned.
    pub(crate) fn spawn(self) -> io::Result<()> {
        thread::Builder::new()
            .name("log-throttle".to_owned())
            .spawn(move || loop {
                thread::sleep(self.state.window);
                self.report();
            })
            .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Debug;
    use std::sync::Mutex;

    use tracing::field::{Field, Visit};
    use tracing::subscriber::with_default;
    use tracing_subscriber::layer::{Context as LayerContext, SubscriberExt};
    use tracing_subscriber::Layer;

    use super::*;

    /// One captured event: its target and its `dropped` field, if any.
    type Captured = (String, Option<u64>);

    /// Records every event that reaches it.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Captured>>>);

    struct DroppedVisitor(Option<u64>);

    impl Visit for DroppedVisitor {
        fn record_u64(&mut self, field: &Field, value: u64) {
            if field.name() == "dropped" {
                self.0 = Some(value);
            }
        }
        fn record_debug(&mut self, _field: &Field, _value: &dyn Debug) {}
    }

    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
            let mut visitor = DroppedVisitor(None);
            event.record(&mut visitor);
            self.0
                .lock()
                .unwrap()
                .push((event.metadata().target().to_owned(), visitor.0));
        }
    }

    impl Capture {
        fn count(&self, target: &str) -> usize {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, _)| t == target)
                .count()
        }
        fn dropped(&self) -> Vec<u64> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(t, _)| t == THROTTLE_TARGET)
                .filter_map(|(_, d)| *d)
                .collect()
        }
    }

    fn with_throttle(
        burst: u64,
        window: Duration,
        body: impl FnOnce(&ThrottleReporter),
    ) -> Capture {
        let capture = Capture::default();
        let throttle = CallsiteThrottle::new(burst, window);
        let reporter = throttle.reporter();
        let subscriber = tracing_subscriber::registry().with(capture.clone().with_filter(throttle));
        with_default(subscriber, || body(&reporter));
        capture
    }

    #[test]
    fn a_callsite_past_its_burst_is_dropped_and_reported() {
        let capture = with_throttle(5, Duration::from_secs(3600), |reporter| {
            for _ in 0..50 {
                tracing::warn!(target: "flood", "retrying");
            }
            assert_eq!(reporter.report(), 1);
            assert_eq!(
                reporter.report(),
                0,
                "a drained count is not reported twice"
            );
        });
        assert_eq!(capture.count("flood"), 5);
        assert_eq!(capture.dropped(), vec![45]);
    }

    #[test]
    fn each_callsite_has_its_own_budget() {
        let capture = with_throttle(3, Duration::from_secs(3600), |_| {
            for _ in 0..10 {
                tracing::warn!(target: "first", "one");
                tracing::warn!(target: "second", "two");
            }
        });
        assert_eq!(capture.count("first"), 3);
        assert_eq!(capture.count("second"), 3);
    }

    #[test]
    fn lines_a_metric_counts_are_never_dropped() {
        let capture = with_throttle(1, Duration::from_secs(3600), |reporter| {
            for _ in 0..20 {
                tracing::error!(target: "errors", "job failed");
                tracing::info!(target: "notify", "business event");
                tracing::info!(target: "events", event = "chat.question_asked", "asked");
            }
            assert_eq!(reporter.report(), 0);
        });
        assert_eq!(capture.count("errors"), 20);
        assert_eq!(capture.count("notify"), 20);
        assert_eq!(capture.count("events"), 20);
    }

    #[test]
    fn a_zero_burst_turns_the_throttle_off() {
        let capture = with_throttle(0, Duration::from_secs(3600), |reporter| {
            for _ in 0..20 {
                tracing::warn!(target: "flood", "retrying");
            }
            assert_eq!(reporter.report(), 0);
        });
        assert_eq!(capture.count("flood"), 20);
    }

    #[test]
    fn a_new_window_restores_the_budget() {
        let capture = with_throttle(2, Duration::from_millis(50), |_| {
            for _ in 0..2 {
                for _ in 0..5 {
                    tracing::warn!(target: "flood", "retrying");
                }
                thread::sleep(Duration::from_millis(120));
            }
        });
        assert_eq!(capture.count("flood"), 4);
    }
}
