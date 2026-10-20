// ABOUTME: Tests the SSRF allowlist guard for user-supplied LLM base_url (T3MP3ST F2)
// ABOUTME: Covers deny-by-default, host matching, IPv6/port/userinfo parsing, metadata block
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for the LLM `base_url` SSRF allowlist helpers (`base_url_allowed`,
//! `base_url_host`) added for T3MP3ST finding F2 (CWE-918): deny-by-default matching,
//! case-insensitivity, and robust host extraction across ports/userinfo/IPv6.

use pierre_routes_admin::llm_settings::{base_url_allowed, base_url_host};

fn allowlist(hosts: &[&str]) -> Vec<String> {
    hosts.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn deny_by_default_empty_allowlist_rejects_everything() {
    // An empty allowlist must reject ANY custom base_url — the core F2 posture.
    assert!(!base_url_allowed("http://localhost:11434/v1", &[]));
    assert!(!base_url_allowed("https://api.example.com/v1", &[]));
    assert!(!base_url_allowed("http://169.254.169.254/", &[]));
}

#[test]
fn allowlisted_host_is_permitted() {
    let list = allowlist(&["localhost", "llm.internal.example.com"]);
    assert!(base_url_allowed("http://localhost:11434/v1", &list));
    assert!(base_url_allowed(
        "https://llm.internal.example.com/v1",
        &list
    ));
}

#[test]
fn non_allowlisted_host_is_rejected() {
    let list = allowlist(&["llm.internal.example.com"]);
    // Cloud metadata + arbitrary hosts not on the list stay blocked.
    assert!(!base_url_allowed(
        "http://169.254.169.254/latest/meta-data/",
        &list
    ));
    assert!(!base_url_allowed("http://[::ffff:169.254.169.254]/", &list));
    assert!(!base_url_allowed("https://evil.example.org/v1", &list));
}

#[test]
fn host_match_is_case_insensitive() {
    let list = allowlist(&["llm.example.com"]);
    assert!(base_url_allowed("https://LLM.Example.COM/v1", &list));
}

#[test]
fn userinfo_and_port_do_not_spoof_the_host() {
    let list = allowlist(&["good.example.com"]);
    // Userinfo before '@' must not be mistaken for the host.
    assert!(base_url_allowed("https://good.example.com:8443/v1", &list));
    assert!(base_url_allowed(
        "https://user:pass@good.example.com/v1",
        &list
    ));
    // A userinfo that *looks* like the allowlisted host must not fool the check.
    assert!(!base_url_allowed(
        "https://good.example.com@evil.example.org/",
        &list
    ));
}

#[test]
fn ipv6_literal_host_is_extracted_and_matched() {
    let list = allowlist(&["::1"]);
    assert!(base_url_allowed("http://[::1]:11434/v1", &list));
    // Same address not on the list → rejected.
    assert!(!base_url_allowed(
        "http://[::1]:11434/v1",
        &allowlist(&["localhost"])
    ));
}

#[test]
fn host_extraction_handles_common_url_shapes() {
    assert_eq!(
        base_url_host("http://localhost:11434/v1").as_deref(),
        Some("localhost")
    );
    assert_eq!(
        base_url_host("https://api.example.com/v1/chat").as_deref(),
        Some("api.example.com")
    );
    assert_eq!(base_url_host("http://[::1]:8080/").as_deref(), Some("::1"));
    assert_eq!(
        base_url_host("https://user:pass@host.example.com:443/x").as_deref(),
        Some("host.example.com")
    );
    assert_eq!(base_url_host("not-a-url"), Some("not-a-url".to_owned()));
    assert_eq!(base_url_host(""), None);
    assert_eq!(base_url_host("http://"), None);
}
