// ABOUTME: Unit tests for pierre-cli's credential cache (serde defaults + save/load/expiry/clear)
// ABOUTME: File-I/O tests redirect $HOME to a throwaway dir so the real ~/.pierre is never touched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;

use pierre_cli::remote::CachedCredentials;
use uuid::Uuid;

// `api.dev.dravr.ai` is deliberately a host that does NOT resolve, and must stay
// that way. These tests only ever treat it as an opaque string, and a fixture host
// guaranteed to be NXDOMAIN means a test that ever starts reaching the network
// fails loudly instead of quietly succeeding against something real. Its twin in
// the CLI's --help was misleading and was corrected; this one is load-bearing.
#[test]
fn deserialize_defaults_token_type_and_skips_none_optionals() {
    let json = r#"{"server":"https://api.dev.dravr.ai","access_token":"abc"}"#;
    let creds: CachedCredentials = serde_json::from_str(json).unwrap();
    assert_eq!(creds.server, "https://api.dev.dravr.ai");
    assert_eq!(creds.access_token, "abc");
    assert_eq!(creds.token_type, "Bearer", "token_type defaults to Bearer");
    assert!(creds.expires_at.is_none());
    assert!(creds.subject.is_none());

    let out = serde_json::to_string(&creds).unwrap();
    assert!(
        !out.contains("expires_at"),
        "None optionals are skipped: {out}"
    );
    assert!(
        !out.contains("subject"),
        "None optionals are skipped: {out}"
    );
}

#[test]
fn save_load_expiry_clear_roundtrip() {
    // Redirect the home dir so we never touch a real ~/.pierre/credentials.json.
    let tmp = env::temp_dir().join(format!("pierre-cli-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&tmp).unwrap();
    env::set_var("HOME", &tmp);

    assert!(
        CachedCredentials::load().unwrap().is_none(),
        "no credentials before login"
    );

    let creds = CachedCredentials {
        server: "https://api.dev.dravr.ai".to_owned(),
        access_token: "tok-123".to_owned(),
        token_type: "Bearer".to_owned(),
        expires_at: None,
        subject: Some("admin@example.com".to_owned()),
    };
    creds.save().unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(CachedCredentials::path().unwrap())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "credential file is owner-read/write only"
        );
    }

    let loaded = CachedCredentials::load()
        .unwrap()
        .expect("credentials present");
    assert_eq!(loaded.access_token, "tok-123");
    assert_eq!(loaded.subject.as_deref(), Some("admin@example.com"));

    // A non-expiring login always satisfies require().
    assert!(CachedCredentials::require(9_999_999_999).is_ok());

    // An expiring login is accepted before, rejected after, its expiry.
    CachedCredentials {
        server: "https://api.dev.dravr.ai".to_owned(),
        access_token: "tok-exp".to_owned(),
        token_type: "Bearer".to_owned(),
        expires_at: Some(1_000),
        subject: None,
    }
    .save()
    .unwrap();
    assert!(
        CachedCredentials::require(500).is_ok(),
        "valid before expiry"
    );
    assert!(
        CachedCredentials::require(2_000).is_err(),
        "expired login is rejected"
    );

    assert!(
        CachedCredentials::clear().unwrap(),
        "clear removes the file"
    );
    assert!(
        CachedCredentials::load().unwrap().is_none(),
        "no credentials after logout"
    );

    let _ = fs::remove_dir_all(&tmp);
}
