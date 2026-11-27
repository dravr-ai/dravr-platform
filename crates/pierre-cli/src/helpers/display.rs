// ABOUTME: Output formatting helpers for pierre-cli
// ABOUTME: Provides consistent display functions for tokens and user information
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::civil_clock::UTC_MINUTE;
use pierre_core::admin::models::GeneratedAdminToken;
use pierre_core::constants::ports::DEFAULT_HTTP_PORT;
use secrecy::ExposeSecret;

/// Display a generated token with important security warnings
pub(crate) fn display_generated_token(token: &GeneratedAdminToken) {
    println!("\nComplete Admin Token Generated Successfully!");
    println!("{}", "=".repeat(80));
    println!("Key TOKEN DETAILS:");
    println!("   Service: {}", token.service_name);
    println!("   Token ID: {}", token.token_id);
    println!(
        "   Super Admin: {}",
        if token.is_super_admin {
            "Success Yes"
        } else {
            "Error No"
        }
    );

    if let Some(expires) = token.expires_at {
        println!("   Expires: {}", expires.format_with(&UTC_MINUTE));
    } else {
        println!("   Expires: Never");
    }

    println!("\nKey YOUR JWT TOKEN (SAVE THIS NOW):");
    println!("WARNING: This token grants admin API access. Do not paste it into shared");
    println!("         terminals, chat threads, or CI logs. It will not be shown again.");
    println!("{}", "=".repeat(80));
    println!("{}", token.jwt_token.expose_secret());
    println!("{}", "=".repeat(80));

    println!("\nWARNING CRITICAL SECURITY NOTES:");
    println!("• This token is shown ONLY ONCE - save it now!");
    println!("• Store it securely in your admin service environment:");
    println!(
        "  export PIERRE_MCP_ADMIN_TOKEN=\"{}\"",
        token.jwt_token.expose_secret()
    );
    println!("• Never share this token or commit it to version control");
    println!("• Use this token in Authorization header: Bearer <token>");
    println!(
        "• This token allows {} API key operations",
        if token.is_super_admin {
            "ALL"
        } else {
            "limited"
        }
    );

    println!("\nDocs NEXT STEPS:");
    println!("1. Save the token to your admin service environment");
    println!("2. Configure your admin service to use this token");
    println!("3. Test the connection with your admin service");

    if !token.is_super_admin {
        println!("4. For super admin access, use --super-admin flag");
    }

    println!("\nUSAGE EXAMPLE:");
    println!(
        "curl -H \"Authorization: Bearer {}\" \\",
        token.jwt_token.expose_secret()
    );
    println!("     -X POST http://localhost:{DEFAULT_HTTP_PORT}/admin/provision \\");
    println!("     -d '{{\"user_email\":\"user@example.com\",\"tier\":\"starter\"}}'");

    println!("\nSuccess Token is ready to use!");
}

/// Display the success message for a plain account `user create --role
/// user` made or re-asserted
pub(crate) fn display_plain_user_success(email: &str, name: &str) {
    println!("\nSuccess User Created Successfully!");
    println!("{}", "=".repeat(50));
    println!("User USER DETAILS:");
    println!("   Email: {email}");
    println!("   Name: {name}");
    println!("   Role: User (no admin access)");
    println!("   Status: Active");

    println!("\nKey LOGIN CREDENTIALS:");
    println!("{}", "=".repeat(50));
    println!("   Email: {email}");
    println!("   Password: (the one you just entered — not echoed for security)");
    println!("\nSuccess User is ready to use!");
}

/// Display admin user creation success message
pub(crate) fn display_admin_user_success(email: &str, name: &str) {
    println!("\nSuccess Admin User Created Successfully!");
    println!("{}", "=".repeat(50));
    println!("User USER DETAILS:");
    println!("   Email: {email}");
    println!("   Name: {name}");
    println!("   Role: Admin");
    println!("   Tier: Enterprise (Full access)");
    println!("   Status: Active");
    println!("   Capabilities: Can impersonate other users");

    println!("\nKey LOGIN CREDENTIALS:");
    println!("{}", "=".repeat(50));
    println!("   Email: {email}");
    println!("   Password: (the one you just entered — not echoed for security)");

    println!("\nWARNING IMPORTANT SECURITY NOTES:");
    println!("• Change the default password in production!");
    println!("• This user has full access to the admin interface");
    println!("• An admin can impersonate any user who is not an admin");
    println!("• Use strong passwords and enable 2FA if available");

    println!("\nDocs NEXT STEPS:");
    println!("1. Start the Pierre MCP Server:");
    println!("   cargo run --bin pierre-mcp-server");
    println!("2. Open the frontend interface (usually http://localhost:8080)");
    println!("3. Login with the credentials above");
    println!("4. Generate your first admin token for API key provisioning");
}
