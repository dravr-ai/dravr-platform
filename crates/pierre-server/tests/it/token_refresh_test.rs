// ABOUTME: Tests the JWT expiry detection the MCP client's automatic refresh relies on
// ABOUTME: Pins the claim layout a client parses: subject and a future expiry
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use chrono::DateTime;

use crate::common;

#[tokio::test]
async fn test_jwt_token_parsing() -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Claims {
        sub: String, // User ID
        exp: i64,    // Expiration timestamp
    }

    println!("🔍 Testing JWT token parsing logic");

    let resources = common::create_test_server_resources().await?;
    let (user_id, user) = common::create_test_user(&resources.agent.database).await?;

    // Generate a JWT token
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)?;
    println!(" Generated JWT token for parsing test");

    // Parse token manually like the MCP client does
    let token_parts: Vec<&str> = token.split('.').collect();
    assert_eq!(token_parts.len(), 3, "JWT should have 3 parts");

    // Decode the payload (middle part)
    let payload = token_parts[1];
    let decoded = general_purpose::URL_SAFE_NO_PAD.decode(payload)?;
    let claims: Claims = serde_json::from_slice(&decoded)?;

    // Verify claims
    assert_eq!(claims.sub, user_id.to_string(), "User ID should match");
    assert!(claims.exp > 0, "Expiration should be set");

    // Verify expiry is in the future
    let now = chrono::Utc::now().timestamp();
    assert!(claims.exp > now, "Token should not be expired");

    println!(" JWT token parsing successful");
    println!("   User ID: {}", claims.sub);
    println!(
        "   Expires at: {}",
        DateTime::from_timestamp(claims.exp, 0).unwrap()
    );

    Ok(())
}
