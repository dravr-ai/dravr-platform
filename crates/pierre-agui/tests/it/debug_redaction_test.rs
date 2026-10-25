// ABOUTME: Pins the AG-UI run subscription's Debug, which prints the replay backlog's size only
// ABOUTME: The backlog holds the athlete's streamed turn; a future derive(Debug) must not print it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_agui::registry::RunSubscription;
use tokio::sync::broadcast;

#[test]
fn a_run_subscription_prints_the_size_of_its_backlog_never_the_events() {
    let receiver = broadcast::channel::<String>(4).1;
    let subscription = RunSubscription {
        backlog: vec![
            r#"{"type":"TEXT_MESSAGE_CONTENT","delta":"sentinel-delta-7f3a"}"#.to_owned(),
            r#"{"type":"TEXT_MESSAGE_CONTENT","delta":"my ferritin is 9"}"#.to_owned(),
        ],
        receiver,
    };
    for printed in [format!("{subscription:?}"), format!("{subscription:#?}")] {
        assert!(!printed.contains("sentinel-delta-7f3a"), "{printed}");
        assert!(!printed.contains("ferritin"), "{printed}");
    }
    let compact = format!("{subscription:?}");
    assert!(
        compact.contains("backlog: 2"),
        "the backlog's size prints: {compact}"
    );
}
