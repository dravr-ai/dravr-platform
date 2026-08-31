// ABOUTME: The nightly capture refresh must flag a dead connection, not quietly fetch nothing
// ABOUTME: Pins the flag reaching the database, the snapshot coupling, and the honest budget report
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/admin/diagnostics/capture-staleness` gave a frozen capture a reader. This
//! is the actor: it re-fetches every live connection and, when one's credential
//! turns out to be gone, flips it to `needs_reauth` so the athlete's next turn
//! offers a reconnect link instead of silence.
//!
//! The assertions are deliberately about content and about persisted state. A
//! sweep that reported `flagged` while leaving the connection row `active` would
//! satisfy an `is_ok` test and leave the next turn exactly as silent as the
//! incident that prompted all of this.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use pierre_core::models::{ConnectionStatus, ConnectionType, TenantId};
use pierre_tool_runtime::capture_sweep::{
    refresh_captures, RefreshOutcome, SweepBudget, DEFAULT_CONNECTION_LIMIT,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use uuid::Uuid;

use crate::common::{create_test_server_resources, create_test_user};

/// One athlete with one connection, and the runtime the sweep runs through.
struct Fixture {
    runtime: Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
}

/// Register `provider` for a fresh athlete and hand back the sweep's inputs.
async fn fixture_with_connection(provider: &str) -> Fixture {
    let resources = create_test_server_resources().await.unwrap();
    let (user_id, _user) = create_test_user(&resources.coach.database)
        .await
        .expect("test user");
    let tenants = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user_id)
        .await
        .expect("list tenants");
    let tenant = tenants.first().expect("user has a tenant").id;

    resources
        .common
        .repos
        .provider_connections
        .register_connection(user_id, tenant, provider, &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    Fixture {
        runtime,
        user_id,
        tenant,
    }
}

/// The sweep's whole point: a connection whose credential is gone gets flagged,
/// and the flag lands in the database.
///
/// The athlete here has a registered connection and no token behind it, which is
/// what a lapsed session looks like to the authenticate path — it reports
/// auth-required rather than a transport error. Reporting the flag is not
/// enough: the reconnect link the athlete's next turn offers is rendered off the
/// persisted `status`, so a report-only sweep would change nothing at all.
#[tokio::test]
async fn the_sweep_flags_a_connection_whose_credential_is_gone() {
    let f = fixture_with_connection("strava").await;

    let report = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("refresh report");

    let line = report
        .connections
        .iter()
        .find(|c| c.user_id == f.user_id.to_string() && c.provider == "strava")
        .expect("the connection was walked");
    assert!(
        matches!(&line.outcome, RefreshOutcome::Flagged { reason } if reason == "session_expired"),
        "expected an auth-shaped failure to flag, got {:?}",
        line.outcome
    );
    assert_eq!(report.attempted, 1);
    assert_eq!(report.flagged, 1);
    assert_eq!(report.failed, 0);
    assert!(report.completed, "one connection fits the budget");

    let connections = f
        .runtime
        .repos()
        .provider_connections
        .get_for_user(f.user_id, Some(f.tenant))
        .await
        .unwrap();
    assert_eq!(
        connections[0].status,
        ConnectionStatus::NeedsReauth,
        "the flag must be persisted, not merely reported"
    );
}

/// Flagging a connection drops it out of the snapshot the sweep walks, so the
/// next night does not spend another headless-browser scrape on a connection
/// that cannot succeed until the athlete acts.
///
/// This is the coupling to the staleness reader: both halves read one snapshot,
/// so a connection the reader has stopped counting is also one the refresher has
/// stopped retrying.
#[tokio::test]
async fn a_flagged_connection_is_not_swept_again() {
    let f = fixture_with_connection("sciotte").await;

    let first = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("first sweep");
    assert_eq!(first.attempted, 1, "the live connection was attempted once");
    assert_eq!(first.flagged, 1);

    let second = refresh_captures(&f.runtime, SweepBudget::default())
        .await
        .expect("second sweep");
    assert_eq!(
        second.attempted, 0,
        "a flagged connection must not be re-attempted"
    );
    assert!(
        second.connections.is_empty(),
        "it has left the snapshot entirely, got {:?}",
        second.connections
    );
}

/// A sweep that runs out of time says so, per connection and in the summary.
///
/// Silence about the connections it never reached is the exact failure this
/// whole subsystem exists to end: a report that looked complete while a capture
/// went untouched would put the blind spot back one level up.
#[tokio::test]
async fn an_exhausted_budget_is_reported_not_hidden() {
    let f = fixture_with_connection("whoop").await;

    let report = refresh_captures(
        &f.runtime,
        SweepBudget {
            per_connection: StdDuration::from_secs(1),
            total: StdDuration::ZERO,
            connection_limit: DEFAULT_CONNECTION_LIMIT,
        },
    )
    .await
    .expect("refresh report");

    assert_eq!(report.attempted, 0, "no fetch may start past the deadline");
    assert_eq!(report.skipped, 1);
    assert!(!report.completed, "the sweep must admit it did not finish");
    let line = report
        .connections
        .first()
        .expect("the connection is listed");
    assert!(
        matches!(line.outcome, RefreshOutcome::SkippedBudgetExhausted),
        "expected an explicit budget skip, got {:?}",
        line.outcome
    );

    let connections = f
        .runtime
        .repos()
        .provider_connections
        .get_for_user(f.user_id, Some(f.tenant))
        .await
        .unwrap();
    assert_eq!(
        connections[0].status,
        ConnectionStatus::Active,
        "a connection the sweep never reached must not be flagged"
    );
}
