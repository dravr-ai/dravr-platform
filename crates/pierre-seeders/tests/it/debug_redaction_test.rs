// ABOUTME: Pins the bootstrap seeder's Debug, which hides the admin and demo passwords it was given
// ABOUTME: A future derive(Debug) on the seed arguments fails here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_seeders::bootstrap::SeedArgs;

#[test]
fn the_bootstrap_arguments_never_print_the_admin_or_demo_password() {
    let args = SeedArgs {
        admin_email: "admin@example.com".to_owned(),
        admin_password: "sentinel-password-7f3a".to_owned(),
        demo_password: Some("sentinel-demo-password-9c1e".to_owned()),
    };
    for printed in [format!("{args:?}"), format!("{args:#?}")] {
        assert!(!printed.contains("sentinel-password-7f3a"), "{printed}");
        assert!(
            !printed.contains("sentinel-demo-password-9c1e"),
            "{printed}"
        );
        assert!(printed.contains("[REDACTED]"), "{printed}");
        assert!(printed.contains("admin@example.com"), "{printed}");
    }
}
