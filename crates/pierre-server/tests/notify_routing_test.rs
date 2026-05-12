// ABOUTME: Unit tests for ContremaitreRoutingProvider — YAML parsing, sparse
// ABOUTME: override merge, dedup/batch/env-filter semantics, schema validation.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "contremaitre")]
mod tests {
    use dravr_tronc::notify::RoutingProvider;
    use pierre_mcp_server::contremaitre::notify_routing::ContremaitreRoutingProvider;
    use std::time::Duration;

    const SAMPLE_YAML: &str = r##"
version: 1
defaults:
  channel: "#dravr-events"
  enabled: true
  sample_rate: 1.0
events:
  user.login:
    channel: "#dravr-pulse"
    dedup_per: [user_id]
    dedup_window: 1h
  embacle.call_started:
    enabled_envs: [dev]
  embacle.call_completed:
    sample_rate: 0.1
  provider.fetch_started:
    batch: 5m
"##;

    #[test]
    fn reload_installs_rules_from_yaml() {
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(SAMPLE_YAML).expect("parse sample YAML"); // Safe: test assertion

        let login = provider.route_for("user.login").expect("user.login routed"); // Safe: test assertion
        assert_eq!(login.channel, "#dravr-pulse");
        let dedup = login.dedup.expect("login has dedup rule"); // Safe: test assertion
        assert_eq!(dedup.keys, vec!["user_id".to_owned()]);
        assert_eq!(dedup.window, Duration::from_secs(60 * 60));
    }

    #[test]
    fn unknown_event_inherits_defaults() {
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(SAMPLE_YAML).expect("parse sample YAML"); // Safe: test assertion

        let rule = provider.route_for("brand.new.event").expect("default rule"); // Safe: test assertion
        assert_eq!(rule.channel, "#dravr-events");
        assert!(rule.enabled);
        assert!(rule.dedup.is_none());
    }

    #[test]
    fn bootstrap_returns_none_until_reload() {
        let provider = ContremaitreRoutingProvider::new_empty();
        // Before reload, defaults.enabled = false, so every event drops.
        assert!(provider.route_for("user.login").is_none());
    }

    #[test]
    fn sample_rate_override_applies() {
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(SAMPLE_YAML).expect("parse sample YAML"); // Safe: test assertion

        let rule = provider
            .route_for("embacle.call_completed")
            .expect("rule present"); // Safe: test assertion
        assert!((rule.sample_rate - 0.1).abs() < f32::EPSILON);
    }

    #[test]
    fn enabled_envs_override_replaces_defaults() {
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(SAMPLE_YAML).expect("parse sample YAML"); // Safe: test assertion

        let rule = provider
            .route_for("embacle.call_started")
            .expect("rule present"); // Safe: test assertion
        let envs = rule.enabled_envs.expect("enabled_envs set"); // Safe: test assertion
        assert_eq!(envs, vec!["dev".to_owned()]);
    }

    #[test]
    fn batch_override_parses_duration() {
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(SAMPLE_YAML).expect("parse sample YAML"); // Safe: test assertion

        let rule = provider
            .route_for("provider.fetch_started")
            .expect("rule present"); // Safe: test assertion
        let batch = rule.batch.expect("batch rule set"); // Safe: test assertion
        assert_eq!(batch.interval, Duration::from_secs(5 * 60));
    }

    #[test]
    fn unsupported_version_is_rejected() {
        let provider = ContremaitreRoutingProvider::new_empty();
        let yaml = "version: 99\nevents: {}";
        let err = provider.reload(yaml).expect_err("rejected"); // Safe: test assertion
        assert!(format!("{err}").contains("unsupported version"));
    }

    #[test]
    fn dedup_per_without_window_rejected() {
        let provider = ContremaitreRoutingProvider::new_empty();
        let yaml = "version: 1\nevents:\n  user.login:\n    dedup_per: [user_id]\n";
        let err = provider.reload(yaml).expect_err("rejected"); // Safe: test assertion
        assert!(format!("{err}").contains("dedup_window"));
    }

    #[test]
    fn sample_rate_out_of_range_rejected() {
        let provider = ContremaitreRoutingProvider::new_empty();
        let yaml = "version: 1\nevents:\n  user.login:\n    sample_rate: 10.0\n";
        let err = provider.reload(yaml).expect_err("rejected"); // Safe: test assertion
        assert!(format!("{err}").contains("sample_rate"));
    }

    #[test]
    fn duration_suffixes_parse_via_reload() {
        // Covers the s/m/h/d suffix logic without exposing the private
        // parse_duration helper. Each duration is read back through the
        // public RoutingRule the layer would see at runtime.
        for (suffix, expected) in [
            ("30s", Duration::from_secs(30)),
            ("5m", Duration::from_secs(5 * 60)),
            ("2h", Duration::from_secs(2 * 60 * 60)),
            ("1d", Duration::from_secs(24 * 60 * 60)),
        ] {
            let yaml = format!(
                "version: 1\nevents:\n  user.login:\n    dedup_per: [user_id]\n    dedup_window: {suffix}\n"
            );
            let provider = ContremaitreRoutingProvider::new_empty();
            provider.reload(&yaml).expect("parse YAML"); // Safe: test assertion
            let rule = provider.route_for("user.login").expect("user.login rule"); // Safe: test assertion
            let dedup = rule.dedup.expect("dedup rule"); // Safe: test assertion
            assert_eq!(dedup.window, expected, "suffix {suffix}");
        }
    }

    #[test]
    fn bare_integer_duration_rejected() {
        let provider = ContremaitreRoutingProvider::new_empty();
        let yaml =
            "version: 1\nevents:\n  user.login:\n    dedup_per: [user_id]\n    dedup_window: 60\n";
        let err = provider.reload(yaml).expect_err("rejected"); // Safe: test assertion
        let msg = format!("{err}");
        assert!(
            msg.contains("duration") || msg.contains("suffix"),
            "expected duration-parse error, got: {msg}"
        );
    }

    #[test]
    fn vendored_yaml_parses() {
        // Smoke test against the shipped routing config so schema drift
        // breaks this test the moment the YAML is bumped.
        let yaml = include_str!("../../../vendor/contremaitre/config/notify-routing.yaml");
        let provider = ContremaitreRoutingProvider::new_empty();
        provider.reload(yaml).expect("vendored YAML parses"); // Safe: test assertion
        assert!(provider.route_for("user.login").is_some());
    }
}
