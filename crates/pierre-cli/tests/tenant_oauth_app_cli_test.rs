// ABOUTME: `tenant set-oauth-app` stores a tenant's provider OAuth app through the real binary
// ABOUTME: Pins that the stdin secret round-trips through the encrypted read path and is never echoed

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::env;
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use pierre_auth::key_management::KeyManager;
#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::{Tenant, TenantId, User};
use pierre_database::backends::{factory::Database, DatabaseProvider};
use uuid::Uuid;

/// Base64 of 32 fixed bytes — the local KEK both this process and the binary use.
const MASTER_KEY: &str = "MDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWY=";
const SECRET: &str = "whoop-client-secret-7c1e";

fn cli_binary() -> String {
    env::var("CARGO_BIN_EXE_pierre-cli")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_pierre-cli").to_owned())
}

/// Open the database the binary will write, with the same key hierarchy.
async fn open_database(url: &str) -> Database {
    env::set_var("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY);
    let (mut key_manager, key) = KeyManager::bootstrap().unwrap();
    let mut database = Database::new(
        url,
        key.to_vec(),
        #[cfg(feature = "postgresql")]
        &PostgresPoolConfig::default(),
    )
    .await
    .unwrap();
    key_manager
        .complete_initialization(&mut database)
        .await
        .unwrap();
    database.migrate().await.unwrap();
    database
}

/// Run `tenant set-oauth-app` with `stdin` piped in.
fn set_oauth_app(url: &str, email: &str, tenant: TenantId, stdin: &str) -> (i32, String, String) {
    let mut child = Command::new(cli_binary())
        .env("DATABASE_URL", url)
        .env("PIERRE_MASTER_ENCRYPTION_KEY", MASTER_KEY)
        .args([
            "tenant",
            "set-oauth-app",
            "--email",
            email,
            "--tenant-id",
            &tenant.to_string(),
            "--provider",
            "WHOOP",
            "--client-id",
            "whoop-app-42",
            "--redirect-uri",
            "https://dravr.example/api/oauth/callback/whoop",
            "--scopes",
            "read:recovery,read:sleep",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[tokio::test]
async fn set_oauth_app_stores_a_secret_the_read_path_decrypts() {
    let path = env::temp_dir().join(format!("pierre-cli-oauth-app-{}.db", Uuid::new_v4()));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let database = open_database(&url).await;
    let repos = database.repositories();

    let email = format!("{}@test.local", Uuid::new_v4());
    let user = User::new(email.clone(), "hash".to_owned(), None);
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "OAuth Tenant".to_owned(),
        format!("tenant-{}", Uuid::new_v4()),
        None,
        "starter".to_owned(),
        user.id,
    );
    repos.tenants.create(&tenant).await.unwrap();

    // No secret on stdin: refused, and nothing is stored.
    let (code, _stdout, stderr) = set_oauth_app(&url, &email, tenant.id, "");
    assert_ne!(code, 0, "an empty secret must be refused");
    assert!(stderr.contains("no client secret on stdin"), "{stderr}");
    assert!(repos
        .tenants
        .get_oauth_credentials(tenant.id, "whoop")
        .await
        .unwrap()
        .is_none());

    let (code, stdout, stderr) = set_oauth_app(&url, &email, tenant.id, &format!("{SECRET}\n"));
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    assert!(
        stdout.contains("whoop OAuth app whoop-app-42 stored"),
        "{stdout}"
    );
    assert!(
        !stdout.contains(SECRET) && !stderr.contains(SECRET),
        "the secret is never echoed"
    );

    let stored = repos
        .tenants
        .get_oauth_credentials(tenant.id, "whoop")
        .await
        .unwrap()
        .expect("the connect flow's read path finds the app");
    assert_eq!(stored.client_secret, SECRET);
    assert_eq!(stored.client_id, "whoop-app-42");
    assert_eq!(
        stored.redirect_uri,
        "https://dravr.example/api/oauth/callback/whoop"
    );
    assert_eq!(stored.scopes, vec!["read:recovery", "read:sleep"]);
    assert!(stored.rate_limit_per_day > 0);

    drop(database);
    let _ = fs::remove_file(&path);
}
