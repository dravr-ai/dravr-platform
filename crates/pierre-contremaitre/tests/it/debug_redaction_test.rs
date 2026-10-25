// ABOUTME: Pins contremaitre's hand-written Debug impls that hide the GitHub PAT and webhook secret
// ABOUTME: A future derive(Debug) on the sync config or the contents client fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Debug;

use pierre_contremaitre::github::GitHubContentsClient;
use pierre_contremaitre::ContremaitreConfig;

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

#[test]
fn the_contremaitre_config_never_prints_its_pat_or_webhook_secret() {
    let config = ContremaitreConfig {
        repo: "dravr-ai/dravr-contremaitre".to_owned(),
        branch: "main".to_owned(),
        github_pat: "github_pat_sentinel-7f3a".to_owned(),
        gcs_bucket: Some("prompts-bucket".to_owned()),
        webhook_secret: "sentinel-webhook-7f3a".to_owned(),
    };
    let printed = printed_without(
        &config,
        &["github_pat_sentinel-7f3a", "sentinel-webhook-7f3a"],
    );
    assert_eq!(printed.matches("[REDACTED]").count(), 2, "{printed}");
    assert!(printed.contains("dravr-ai/dravr-contremaitre"), "{printed}");
}

#[test]
fn the_github_contents_client_never_prints_its_pat() {
    let client = GitHubContentsClient::new(
        "dravr-ai/dravr-contremaitre".to_owned(),
        "main".to_owned(),
        "github_pat_sentinel-7f3a".to_owned(),
    );
    let printed = printed_without(&client, &["github_pat_sentinel-7f3a"]);
    assert!(printed.contains("pat: \"[REDACTED]\""), "{printed}");
}
