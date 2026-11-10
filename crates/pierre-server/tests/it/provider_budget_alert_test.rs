// ABOUTME: The spent-budget line provider_budget_monitoring.tf alerts on is the one the limiter logs for a foreground read
// ABOUTME: Drives a real refusal in each share and compares target, message and fields with the Terraform filter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1491. A log-based alert reads the target and words of a log line.
//! Reworded or moved on one side only, the alert never fires and nothing says
//! so, so the test reads the filter out of the Terraform and makes the
//! limiter log the line: a foreground refusal must match it, and a
//! background one, the share doing its job, must not.

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use pierre_providers::request_budget::{
    in_background, ProviderRateLimiter, RequestBudget, ONE_DAY,
};
use pierre_test_support::db::create_test_db;
use regex::Regex;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

/// The app the refused requests are signed by.
const APP: &str = "alert-test-app";

/// One captured log line: its target, message and the names of its fields.
#[derive(Debug, Clone)]
struct Line {
    target: String,
    message: String,
    fields: Vec<String>,
}

#[derive(Default)]
struct LineCapture {
    lines: Arc<Mutex<Vec<Line>>>,
}

struct LineVisitor<'a> {
    message: &'a mut String,
    fields: &'a mut Vec<String>,
}

impl Visit for LineVisitor<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            *self.message = format!("{value:?}");
        } else {
            self.fields.push(field.name().to_owned());
        }
    }
}

impl<S: Subscriber> Layer<S> for LineCapture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = String::new();
        let mut fields = Vec::new();
        event.record(&mut LineVisitor {
            message: &mut message,
            fields: &mut fields,
        });
        self.lines.lock().unwrap().push(Line {
            target: event.metadata().target().to_owned(),
            message,
            fields,
        });
    }
}

/// What the `provider_budget_spent` metric matches: the `rust.target` label,
/// the exact message, and the payload fields its labels are extracted from.
struct AlertFilter {
    target: String,
    message: String,
    fields: Vec<String>,
}

impl AlertFilter {
    fn matches(&self, line: &Line) -> bool {
        line.target == self.target && line.message == self.message
    }
}

fn alert_filter() -> AlertFilter {
    // Read as data, not code: the Terraform filter and the log line are joined
    // by nothing at runtime, so this parity check reads one side and makes the
    // limiter emit the other.
    let terraform =
        include_str!("../../../../infra/environments/dev/provider_budget_monitoring.tf");
    let metric =
        Regex::new(r#"(?s)resource "google_logging_metric" "provider_budget_spent" \{(.*?)\n\}"#)
            .unwrap()
            .captures(terraform)
            .expect("provider_budget_monitoring.tf has no provider_budget_spent metric")[1]
            .to_owned();
    let capture = |pattern: &str, what: &str| {
        Regex::new(pattern)
            .unwrap()
            .captures(&metric)
            .unwrap_or_else(|| panic!("the provider_budget_spent metric has no {what}"))[1]
            .to_owned()
    };
    let fields = Regex::new(r"EXTRACT\(jsonPayload\.(\w+)\)")
        .unwrap()
        .captures_iter(&metric)
        .map(|c| c[1].to_owned())
        .collect();
    AlertFilter {
        target: capture(r#"labels\."rust\.target"="([^"]+)""#, "rust.target filter"),
        message: capture(r#"jsonPayload\.message="([^"]+)""#, "message filter"),
        fields,
    }
}

#[tokio::test]
async fn the_alert_counts_the_foreground_refusal_the_limiter_logs_and_only_that() {
    let filter = alert_filter();
    assert!(
        !filter.fields.is_empty(),
        "the provider_budget_spent metric extracts no label from the payload"
    );
    let capture = LineCapture::default();
    let lines = Arc::clone(&capture.lines);
    let _guard = tracing_subscriber::registry().with(capture).set_default();

    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let limiter = Arc::new(ProviderRateLimiter::new(
        Arc::clone(&repos.usage_counters),
        Arc::clone(&repos.provider_connections),
    ));
    // A day of four: one background request (25%) spends the background
    // share, three more spend the day.
    limiter.set_budgets("strava", &[(4, ONE_DAY)]);
    let budget = RequestBudget::new(limiter, APP.to_owned(), None);

    in_background(budget.admit("strava"))
        .await
        .expect("the first background request fits the background share");
    in_background(budget.admit("strava"))
        .await
        .expect_err("a background request past its share is refused");
    assert!(
        !lines
            .lock()
            .unwrap()
            .iter()
            .any(|line| filter.matches(line)),
        "bug: a background refusal, the share doing its job, would page: {:?}",
        lines.lock().unwrap()
    );

    for _ in 0..3 {
        budget
            .admit("strava")
            .await
            .expect("a foreground request keeps the rest of the day");
    }
    budget
        .admit("strava")
        .await
        .expect_err("a foreground request past the day is refused");

    let paged: Vec<Line> = lines
        .lock()
        .unwrap()
        .iter()
        .filter(|line| filter.matches(line))
        .cloned()
        .collect();
    assert_eq!(
        paged.len(),
        1,
        "bug: the foreground refusal no longer logs the line provider_budget_monitoring.tf \
         alerts on ({} / {:?}): {:?}",
        filter.target,
        filter.message,
        lines.lock().unwrap()
    );
    for field in &filter.fields {
        assert!(
            paged[0].fields.contains(field),
            "bug: the refusal line carries no `{field}`, which the alert extracts as a label: {:?}",
            paged[0]
        );
    }
}
