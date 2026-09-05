// ABOUTME: Pins how the messaging turn runner is selected — in-process by default, Cloud Tasks only fully configured
// ABOUTME: Drives TurnRunner::parse with a settings map so the boot-time refusals are asserted without touching the environment
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The runner is chosen once at boot from the environment (registre#126). A
//! half-configured Cloud Tasks runner would enqueue turns nowhere, so the
//! selection refuses it by name; everything else about the selection — the
//! task target, the task name, the dispatch deadline derived from the turn
//! watchdog, the claim wait — is pinned here so a change to any of them is a
//! change someone made on purpose.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::gcp_token::TokenProvider;
use pierre_mcp_server::services::turn_runner::{
    TurnRunner, ENV_TURN_CLAIM_WAIT_SECS, ENV_TURN_OIDC_SERVICE_ACCOUNT, ENV_TURN_QUEUE,
    ENV_TURN_RUNNER, ENV_TURN_TARGET_URL,
};

struct StaticToken;

#[async_trait]
impl TokenProvider for StaticToken {
    async fn access_token(&self) -> AppResult<String> {
        Ok("ya29.static".to_owned())
    }
}

const WATCHDOG: Duration = Duration::from_mins(16);
const QUEUE: &str =
    "projects/dravr-dev/locations/northamerica-northeast1/queues/dravr-mcp-server-turns";
const TARGET: &str = "https://dravr-mcp-server-api-123456.northamerica-northeast1.run.app";
const SA: &str = "dravr-app@dravr-dev.iam.gserviceaccount.com";

fn full() -> HashMap<&'static str, String> {
    HashMap::from([
        (ENV_TURN_RUNNER, "cloud_tasks".to_owned()),
        (ENV_TURN_QUEUE, QUEUE.to_owned()),
        (ENV_TURN_TARGET_URL, format!("{TARGET}/")),
        (ENV_TURN_OIDC_SERVICE_ACCOUNT, SA.to_owned()),
    ])
}

fn parse(settings: &HashMap<&str, String>, watchdog: Duration) -> AppResult<TurnRunner> {
    TurnRunner::parse(
        |key| settings.get(key).cloned(),
        watchdog,
        Arc::new(StaticToken),
    )
}

#[test]
fn no_setting_at_all_runs_turns_in_process() {
    let runner = parse(&HashMap::new(), WATCHDOG).unwrap();
    assert!(matches!(runner, TurnRunner::InProcess));
    assert_eq!(runner.label(), "in_process");
    assert!(runner.cloud_tasks().is_none());
}

#[test]
fn a_fully_configured_cloud_tasks_runner_knows_its_target_name_and_deadline() {
    let runner = parse(&full(), WATCHDOG).unwrap();
    assert_eq!(runner.label(), "cloud_tasks");
    let cloud = runner.cloud_tasks().expect("cloud tasks runner");

    // The trailing slash on the configured target is dropped, so the task
    // URL and the token audience are one exact string.
    assert_eq!(
        cloud.run_url("row-1"),
        format!("{TARGET}/internal/turns/row-1/run")
    );
    assert_eq!(cloud.verifier().audience(), TARGET);
    assert_eq!(cloud.verifier().service_account(), SA);

    // Watchdog plus a minute: Cloud Tasks never gives up on a turn the
    // watchdog would still let finish.
    assert_eq!(cloud.dispatch_deadline(), Duration::from_mins(17));
    assert_eq!(cloud.claim_wait(), Duration::from_mins(4));

    let name = cloud.task_name("row-1", 0);
    assert!(
        name.starts_with(&format!("{QUEUE}/tasks/")),
        "task names live under the queue: {name}"
    );
    assert!(
        name.ends_with("-row-1-e0"),
        "the row id and the enqueue sequence close the name: {name}"
    );
    assert_ne!(
        cloud.task_name("row-1", 1),
        name,
        "a re-enqueue is a new name, because an executed name is unusable for a day"
    );
    assert_eq!(
        cloud.task_name("row-1", 0),
        name,
        "the same enqueue is the same name, so a retried create is deduplicated"
    );
}

#[test]
fn the_claim_wait_is_configurable_in_seconds() {
    let mut settings = full();
    settings.insert(ENV_TURN_CLAIM_WAIT_SECS, "15".to_owned());
    let runner = parse(&settings, WATCHDOG).unwrap();
    assert_eq!(
        runner.cloud_tasks().unwrap().claim_wait(),
        Duration::from_secs(15)
    );

    settings.insert(ENV_TURN_CLAIM_WAIT_SECS, "soon".to_owned());
    let err = parse(&settings, WATCHDOG).unwrap_err();
    assert!(
        err.message.contains(ENV_TURN_CLAIM_WAIT_SECS),
        "the refusal names the setting: {}",
        err.message
    );
}

#[test]
fn a_cloud_tasks_runner_missing_any_of_its_three_settings_is_refused_by_name() {
    for missing in [
        ENV_TURN_QUEUE,
        ENV_TURN_TARGET_URL,
        ENV_TURN_OIDC_SERVICE_ACCOUNT,
    ] {
        let mut settings = full();
        settings.remove(missing);
        let err = parse(&settings, WATCHDOG).unwrap_err();
        assert!(
            err.message.contains(missing),
            "removing {missing} must be refused by name, got: {}",
            err.message
        );

        // Present but blank is the same as absent.
        let mut blank = full();
        blank.insert(missing, "  ".to_owned());
        let err = parse(&blank, WATCHDOG).unwrap_err();
        assert!(
            err.message.contains(missing),
            "blank {missing}: {}",
            err.message
        );
    }
}

#[test]
fn an_unknown_runner_is_refused() {
    let settings = HashMap::from([(ENV_TURN_RUNNER, "pubsub".to_owned())]);
    let err = parse(&settings, WATCHDOG).unwrap_err();
    assert!(err.message.contains("pubsub"), "{}", err.message);
    assert!(err.message.contains(ENV_TURN_RUNNER), "{}", err.message);
}

#[test]
fn a_watchdog_too_long_for_a_cloud_tasks_deadline_is_refused() {
    // Thirty minutes is the ceiling Cloud Tasks accepts; the watchdog plus a
    // minute of margin has to fit under it.
    let err = parse(&full(), Duration::from_secs(29 * 60 + 1)).unwrap_err();
    assert!(
        err.message.contains("MESSAGING_TURN_WATCHDOG_SECS"),
        "the refusal names the knob: {}",
        err.message
    );
    assert!(parse(&full(), Duration::from_mins(29)).is_ok());
}
