// ABOUTME: Pins the device login's Debug, which hides the super-admin bearer token it minted
// ABOUTME: A future derive(Debug) on the login result fails here instead of printing the token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_cli::remote::device_flow::DeviceLogin;

#[test]
fn a_device_login_never_prints_its_access_token() {
    let login = DeviceLogin {
        access_token: "sentinel-admin-token-7f3a".to_owned(),
        token_type: "Bearer".to_owned(),
        subject: Some("operator@dravr.ai".to_owned()),
    };
    for printed in [format!("{login:?}"), format!("{login:#?}")] {
        assert!(!printed.contains("sentinel-admin-token-7f3a"), "{printed}");
        assert!(printed.contains("[REDACTED]"), "{printed}");
        assert!(printed.contains("operator@dravr.ai"), "{printed}");
    }
}
