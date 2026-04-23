// ABOUTME: Integration tests for the AG-UI protocol module in pierre-server
// ABOUTME: Exercises event wire format, filter semantics, and RunRegistry fan-out
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::agui::emitter::{AgUiSink, BroadcastSink, NoopSink};
use pierre_mcp_server::agui::events::AgUiEventKind;
use pierre_mcp_server::agui::{
    AgUiEvent, AgUiEventFilter, AuthorizedSubscribe, PublishOutcome, RunOwner, RunRegistry,
};
use pierre_mcp_server::models::TenantId;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Synthetic owner for sink-level tests — real runs bind to the
/// authenticated `user_id` and JWT `active_tenant_id`.
fn test_owner() -> RunOwner {
    RunOwner::new(Uuid::new_v4(), TenantId::new())
}

/// Events serialize with the AG-UI spec's screaming-snake-case `type` tag
/// so the wire format is interoperable with any AG-UI client.
#[tokio::test]
async fn run_started_serializes_with_spec_tag() {
    let event = AgUiEvent::run_started("run_abc", Some("thread_xyz"));
    let Ok(payload) = serde_json::to_value(&event) else {
        panic!("AG-UI event must serialize to JSON");
    };
    assert_eq!(payload["type"], "RUN_STARTED");
    assert_eq!(payload["run_id"], "run_abc");
    assert_eq!(payload["thread_id"], "thread_xyz");
}

/// Events round-trip through JSON so downstream AG-UI consumers can
/// re-serialize them verbatim.
#[tokio::test]
async fn events_round_trip_via_json() {
    let event = AgUiEvent::step_finished("run_abc", "dispatch");
    let Ok(json) = serde_json::to_string(&event) else {
        panic!("AG-UI event must serialize to JSON");
    };
    let Ok(decoded) = serde_json::from_str::<AgUiEvent>(&json) else {
        panic!("AG-UI event must deserialize from its own JSON");
    };
    assert!(matches!(decoded.kind(), AgUiEventKind::StepFinished));
    assert_eq!(decoded.run_id(), "run_abc");
}

/// Filters gate which event kinds a sink forwards. `without` narrows the
/// default allowlist; unwanted kinds are silently dropped inside the sink.
#[tokio::test]
async fn filter_without_narrows_allowlist() {
    let filter = AgUiEventFilter::allow_all().without(AgUiEventKind::TextMessageContent);
    assert!(filter.allows(AgUiEventKind::RunStarted));
    assert!(!filter.allows(AgUiEventKind::TextMessageContent));
}

/// Registry `subscribe` returns `None` for unknown runs so SSE handlers
/// can emit a 404 rather than hang forever.
#[tokio::test]
async fn registry_subscribe_returns_none_for_unknown_run() {
    let registry = RunRegistry::new();
    assert!(registry.subscribe_self("run_that_does_not_exist").is_none());
}

/// A registered run exposes subscribers that receive every event the
/// sink publishes. `RunFinished` closes the channel, terminating
/// downstream streams cleanly.
#[tokio::test]
async fn broadcast_sink_fans_events_to_subscribers() {
    let registry = RunRegistry::new();
    let run_id = "run_fanout_test";
    let _producer = registry.register(run_id, test_owner());
    let Some(mut subscription) = registry.subscribe_self(run_id) else {
        panic!("subscribe must succeed for a registered run");
    };
    let sink = BroadcastSink::new(registry.clone(), AgUiEventFilter::default());

    sink.emit(&AgUiEvent::run_started(run_id, None)).await;
    sink.emit(&AgUiEvent::run_finished(run_id)).await;

    let Ok(first) = subscription.receiver.recv().await else {
        panic!("subscriber must receive RUN_STARTED");
    };
    assert!(first.contains("RUN_STARTED"));
    let Ok(second) = subscription.receiver.recv().await else {
        panic!("subscriber must receive RUN_FINISHED");
    };
    assert!(second.contains("RUN_FINISHED"));

    registry.unregister(run_id);
}

/// A denied kind is silently dropped by the sink — no channel message,
/// no error.
#[tokio::test]
async fn broadcast_sink_respects_filter_denies() {
    let registry = RunRegistry::new();
    let run_id = "run_filter_test";
    let _producer = registry.register(run_id, test_owner());
    let Some(mut subscription) = registry.subscribe_self(run_id) else {
        panic!("subscribe must succeed");
    };
    let filter = AgUiEventFilter::deny_all().with(AgUiEventKind::RunFinished);
    let sink = BroadcastSink::new(registry.clone(), filter);

    sink.emit(&AgUiEvent::run_started(run_id, None)).await;
    sink.emit(&AgUiEvent::run_finished(run_id)).await;

    let Ok(only_event) = subscription.receiver.recv().await else {
        panic!("subscriber must receive the allowed event");
    };
    assert!(only_event.contains("RUN_FINISHED"));
    assert!(!only_event.contains("RUN_STARTED"));

    registry.unregister(run_id);
}

/// The no-op sink swallows every event regardless of filter.
#[tokio::test]
async fn noop_sink_drops_events() {
    let sink = NoopSink::new(AgUiEventFilter::allow_all());
    sink.emit(&AgUiEvent::run_started("run_1", None)).await;
}

/// A late subscriber receives the replay backlog before live events.
///
/// A client that subscribes *after* the pipeline has already started
/// emitting receives the replay backlog, in order, before it sees
/// any live events. This is the reconnection / late-attach case the
/// P1 concern called out.
#[tokio::test]
async fn late_subscribers_receive_replay_backlog() {
    let registry = RunRegistry::new();
    let run_id = "run_replay_test";
    let _producer = registry.register(run_id, test_owner());
    let sink = BroadcastSink::new(registry.clone(), AgUiEventFilter::default());

    // Emit three events BEFORE any subscriber exists.
    sink.emit(&AgUiEvent::run_started(run_id, Some("thread_1")))
        .await;
    sink.emit(&AgUiEvent::step_started(run_id, "prompt_assembly"))
        .await;
    sink.emit(&AgUiEvent::step_finished(run_id, "prompt_assembly"))
        .await;

    // Subscribe *after* those events.
    let Some(mut subscription) = registry.subscribe_self(run_id) else {
        panic!("subscribe must succeed");
    };
    assert_eq!(subscription.backlog.len(), 3);
    assert!(subscription.backlog[0].contains("RUN_STARTED"));
    assert!(subscription.backlog[1].contains("STEP_STARTED"));
    assert!(subscription.backlog[2].contains("STEP_FINISHED"));

    // Subsequent events still flow over the live receiver.
    sink.emit(&AgUiEvent::run_finished(run_id)).await;
    let Ok(live) = subscription.receiver.recv().await else {
        panic!("live receiver must yield RUN_FINISHED");
    };
    assert!(live.contains("RUN_FINISHED"));

    registry.unregister(run_id);
}

/// `authorize_and_subscribe` makes owner check + subscribe atomic.
///
/// The two steps run together under the registry shard guard so a
/// concurrent `unregister` + `register` with a different owner
/// cannot leak a stream against a slot the caller does not own.
#[tokio::test]
async fn authorize_and_subscribe_returns_forbidden_for_other_owner() {
    let registry = RunRegistry::new();
    let run_id = "run_toctou_test";
    let owner = test_owner();
    let other = test_owner();
    let _producer = registry.register(run_id, owner);

    match registry.authorize_and_subscribe(run_id, other) {
        AuthorizedSubscribe::Forbidden => {}
        AuthorizedSubscribe::Ok(_) => panic!("must not return Ok for non-owner"),
        AuthorizedSubscribe::NotFound => panic!("run is registered, expected Forbidden"),
    }

    // Same owner gets through.
    assert!(matches!(
        registry.authorize_and_subscribe(run_id, owner),
        AuthorizedSubscribe::Ok(_),
    ));

    // Unknown run is NotFound, not Forbidden.
    assert!(matches!(
        registry.authorize_and_subscribe("does_not_exist", owner),
        AuthorizedSubscribe::NotFound,
    ));
}

/// `publish` returns `NoSlot` for an unregistered run.
///
/// The sink uses that signal to log a warning so operators can spot
/// a missing `register` call instead of silently dropping events.
#[tokio::test]
async fn publish_distinguishes_no_slot_from_no_subscribers() {
    let registry = RunRegistry::new();
    assert_eq!(
        registry.publish("never_registered", "{}".into()),
        PublishOutcome::NoSlot,
    );

    let run_id = "run_no_subs_test";
    let _producer = registry.register(run_id, test_owner());
    // Registered but no subscribers connected yet → NoSubscribers,
    // and the event is buffered for replay.
    assert_eq!(
        registry.publish(run_id, "{}".into()),
        PublishOutcome::NoSubscribers,
    );
    let Some(sub) = registry.subscribe_self(run_id) else {
        panic!("subscribe must succeed");
    };
    assert_eq!(sub.backlog.len(), 1);
}

/// A subscribe taken after a publish returns that event once, via the
/// backlog, and never again on the live receiver (Bug C3 defensive:
/// the snapshot path holds the replay mutex across both `subscribe`
/// and the buffer copy so a concurrent publish cannot land in both at
/// once). Asserts the single-delivery invariant the ordering fix is
/// designed to preserve.
#[tokio::test]
async fn subscribe_after_publish_does_not_redeliver_on_receiver() {
    let registry = RunRegistry::new();
    let run_id = "run_no_redelivery_test";
    let _producer = registry.register(run_id, test_owner());

    // Publish one event, then subscribe — the event MUST appear in the
    // backlog once, and the live receiver MUST NOT yield it.
    assert_eq!(
        registry.publish(run_id, "event_1".into()),
        PublishOutcome::NoSubscribers,
    );

    let Some(mut subscription) = registry.subscribe_self(run_id) else {
        panic!("subscribe must succeed");
    };
    assert_eq!(subscription.backlog, vec!["event_1".to_owned()]);
    match subscription.receiver.try_recv() {
        Err(broadcast::error::TryRecvError::Empty) => {}
        Ok(redelivery) => panic!(
            "live receiver re-delivered backlog event {redelivery:?} — Bug C3 race regression"
        ),
        Err(other) => panic!("unexpected receiver state: {other:?}"),
    }

    // A subsequent publish still reaches the live receiver, so the
    // subscribe lock ordering did not accidentally break live delivery.
    let _ = registry.publish(run_id, "event_2".into());
    let live = subscription
        .receiver
        .recv()
        .await
        .expect("live receiver must yield post-subscribe publish");
    assert_eq!(live, "event_2");

    registry.unregister(run_id);
}

/// `register_scoped` returns an RAII guard that unregisters the run
/// automatically when it goes out of scope — channel adapters that
/// propagate errors early cannot leak registry entries.
#[tokio::test]
async fn run_scope_guard_unregisters_on_drop() {
    let registry = RunRegistry::new();
    let run_id = "run_scope_test";
    {
        let _scope = registry.register_scoped(run_id, test_owner());
        assert!(registry.owner_of(run_id).is_some());
        assert_eq!(registry.active_run_count(), 1);
    } // scope drops here
    assert!(registry.owner_of(run_id).is_none());
    assert_eq!(registry.active_run_count(), 0);
}
