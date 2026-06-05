// ABOUTME: Security module for HTTP security headers and request hardening utilities
// ABOUTME: Provides security header configuration, header auditing, cookies, and CSRF protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Security Module
//!
//! Security features for Pierre MCP Server including:
//! - Security header configuration and auditing
//! - Secure HTTP cookie utilities
//! - CSRF protection

use std::collections::HashMap;
use std::hash::BuildHasher;
use tracing::warn;

/// Secure HTTP cookie utilities
pub mod cookies;
/// CSRF protection token management
pub mod csrf;

/// Security audit helper function
pub fn audit_security_headers<S: BuildHasher>(headers: &HashMap<String, String, S>) -> bool {
    let required_headers = [
        "Content-Security-Policy",
        "X-Frame-Options",
        "X-Content-Type-Options",
    ];

    for header in &required_headers {
        if !headers.contains_key(*header) {
            warn!("Missing required security header: {}", header);
            return false;
        }
    }

    true
}

/// Security header configuration and validation
pub mod headers {
    use pierre_core::constants::time_constants;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    /// Security headers configuration  
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct SecurityConfig {
        /// Environment type (development, production)
        pub environment: String,
        /// Security headers to apply
        pub headers: HashMap<String, String>,
    }

    impl SecurityConfig {
        /// Create development security configuration
        #[must_use]
        pub fn development() -> Self {
            let mut headers = HashMap::new();
            headers.insert("Content-Security-Policy".to_owned(), 
                          "default-src 'self'; script-src 'self' 'unsafe-inline' 'unsafe-eval'; style-src 'self' 'unsafe-inline'".to_owned());
            headers.insert("X-Frame-Options".to_owned(), "DENY".to_owned());
            headers.insert("X-Content-Type-Options".to_owned(), "nosniff".to_owned());
            headers.insert(
                "Referrer-Policy".to_owned(),
                "strict-origin-when-cross-origin".to_owned(),
            );
            headers.insert(
                "Permissions-Policy".to_owned(),
                "camera=(), microphone=(), geolocation=()".to_owned(),
            );

            Self {
                environment: "development".to_owned(),
                headers,
            }
        }

        /// Create production security configuration
        #[must_use]
        pub fn production() -> Self {
            let mut headers = HashMap::new();
            headers.insert(
                "Content-Security-Policy".to_owned(),
                "default-src 'self'; script-src 'self'; style-src 'self'".to_owned(),
            );
            headers.insert("X-Frame-Options".to_owned(), "DENY".to_owned());
            headers.insert("X-Content-Type-Options".to_owned(), "nosniff".to_owned());
            headers.insert("Referrer-Policy".to_owned(), "strict-origin".to_owned());
            headers.insert(
                "Strict-Transport-Security".to_owned(),
                format!(
                    "max-age={}; includeSubDomains",
                    time_constants::SECONDS_PER_YEAR
                ),
            );
            headers.insert(
                "Permissions-Policy".to_owned(),
                "camera=(), microphone=(), geolocation=()".to_owned(),
            );

            Self {
                environment: "production".to_owned(),
                headers,
            }
        }

        /// Create security configuration from environment string
        #[must_use]
        pub fn from_environment(env: &str) -> Self {
            match env.to_lowercase().as_str() {
                "production" | "prod" => Self::production(),
                _ => Self::development(),
            }
        }

        /// Get headers as `HashMap` for HTTP integration
        #[must_use]
        pub const fn to_headers(&self) -> &HashMap<String, String> {
            &self.headers
        }
    }
}
