// ABOUTME: PII-safe logging and redaction middleware for sensitive data protection
// ABOUTME: Filters headers, request bodies, and logs to prevent PII leakage and compliance violations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! PII-safe logging and redaction for compliance and security
//!
//! This module provides:
//! - HTTP header redaction (Authorization, Cookie, X-API-Key, etc.)
//! - JSON body field redaction (`client_secret`, tokens, passwords)
//! - Email address masking for PII protection
//! - Token pattern detection and redaction
//! - Bounded metric labels to prevent Prometheus cardinality explosions
//!
//! ## Usage
//!
//! ```rust
//! use pierre_middleware::redaction::{RedactionConfig, redact_headers, mask_email};
//!
//! let config = RedactionConfig::default();
//! let headers = [
//!     ("authorization", "Bearer secret_token"),
//!     ("content-type", "application/json"),
//! ];
//! let safe_headers = redact_headers(headers, &config);
//! // safe_headers will have authorization redacted
//!
//! let email = "testuser@domain.com";
//! let masked = mask_email(email);
//! // masked will be "t***@d***.com"
//! ```

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use bitflags::bitflags;
use http::Uri;
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

bitflags! {
    /// Redaction feature flags to control which types of data to redact
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RedactionFeatures: u8 {
        /// Redact HTTP headers (Authorization, Cookie, etc.)
        const HEADERS = 0b0001;
        /// Redact JSON body fields (client_secret, tokens, etc.)
        const BODY_FIELDS = 0b0010;
        /// Mask email addresses
        const EMAILS = 0b0100;
        /// Enable all redaction features
        const ALL = Self::HEADERS.bits() | Self::BODY_FIELDS.bits() | Self::EMAILS.bits();
    }
}

/// Configuration for PII redaction
#[derive(Debug, Clone)]
pub struct RedactionConfig {
    /// Enable redaction globally (default: true in production, false in dev)
    pub enabled: bool,
    /// Which redaction features to enable
    pub features: RedactionFeatures,
    /// Replacement string for redacted sensitive data
    pub redaction_placeholder: String,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            features: RedactionFeatures::ALL,
            redaction_placeholder: "[REDACTED]".to_owned(),
        }
    }
}

impl RedactionConfig {
    /// Create redaction config from explicit settings.
    ///
    /// `redact_pii` toggles whether PII redaction is enabled (`true` enables the
    /// full [`RedactionFeatures::ALL`] feature set). `placeholder` is the
    /// replacement string written in place of redacted content. The caller
    /// resolves both from its runtime config — the middleware itself does not
    /// read any global state so it stays decoupled from `ServerConfig`.
    #[must_use]
    pub fn new(redact_pii: bool, placeholder: String) -> Self {
        let features = if redact_pii {
            RedactionFeatures::ALL
        } else {
            RedactionFeatures::empty()
        };

        Self {
            enabled: redact_pii,
            features,
            redaction_placeholder: placeholder,
        }
    }

    /// Check if redaction is disabled
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        !self.enabled
    }
}

/// Sensitive HTTP headers that should be redacted
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "cookie",
    "set-cookie",
    "x-api-key",
    "x-auth-token",
    "x-access-token",
    "api-key",
    "api_key",
    "apikey",
    "proxy-authorization",
    "www-authenticate",
];

/// Sensitive JSON fields that should be redacted
const SENSITIVE_FIELDS: &[&str] = &[
    "password",
    "client_secret",
    "client-secret",
    "access_token",
    "accessToken",
    "refresh_token",
    "refreshToken",
    "api_key",
    "apiKey",
    "api-key",
    "secret",
    "private_key",
    "privateKey",
    "encryption_key",
    "encryptionKey",
    "jwt_secret",
    "jwtSecret",
];

/// Query-string parameters whose value is a secret or a direct identifier.
///
/// The URL query is the one part of a request line that routinely carries live
/// credentials: an `OAuth` authorization `code` and `state` on a provider
/// callback, a password-reset or channel-link `token`, an API key on a
/// machine-to-machine call. All of them are worth exactly as much to an
/// attacker reading a log line as they are to the client that was issued them.
const SENSITIVE_QUERY_PARAMS: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "auth",
    "client_secret",
    "code",
    "email",
    "id_token",
    "jwt",
    "key",
    "link_token",
    "password",
    "refresh_token",
    "secret",
    "session",
    "signature",
    "state",
    "token",
];

/// Redact sensitive HTTP headers
///
/// # Arguments
///
/// * `headers` - Iterator of (name, value) tuples
/// * `config` - Redaction configuration
///
/// # Returns
///
/// Vector of (name, value) tuples with sensitive headers redacted
pub fn redact_headers<'a, I>(headers: I, config: &RedactionConfig) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    if !config.enabled || !config.features.contains(RedactionFeatures::HEADERS) {
        return headers
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
    }

    headers
        .into_iter()
        .map(|(name, value)| {
            let name_lower = name.to_lowercase();
            let redacted_value = if SENSITIVE_HEADERS.contains(&name_lower.as_str()) {
                config.redaction_placeholder.clone()
            } else {
                value.to_owned()
            };
            (name.to_owned(), redacted_value)
        })
        .collect()
}

/// Redact sensitive fields in JSON-like text
///
/// # Arguments
///
/// * `text` - JSON or log text that may contain sensitive fields
/// * `config` - Redaction configuration
///
/// # Returns
///
/// Text with sensitive field values redacted
#[must_use]
pub fn redact_json_fields(text: &str, config: &RedactionConfig) -> String {
    if !config.enabled || !config.features.contains(RedactionFeatures::BODY_FIELDS) {
        return text.to_owned();
    }

    let mut result = text.to_owned();

    for field in SENSITIVE_FIELDS {
        // Match patterns like:
        // "field": "value"
        // "field":"value"
        // field: "value"
        // field = "value"
        let patterns = [
            format!(r#""{field}"\s*:\s*"[^"]*""#),
            format!(r#"{field}\s*:\s*"[^"]*""#),
            format!(r#"{field}\s*=\s*"[^"]*""#),
        ];

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                result = re
                    .replace_all(
                        &result,
                        format!(r#""{field}": "{}""#, config.redaction_placeholder),
                    )
                    .to_string();
            }
        }
    }

    result
}

/// Mask email addresses for PII protection
///
/// Masks email addresses by showing only first characters of local and domain parts
///
/// # Arguments
///
/// * `email` - Email address to mask
///
/// # Returns
///
/// Masked email with first character of local and domain parts visible
#[must_use]
pub fn mask_email(email: &str) -> String {
    email_regex().map_or_else(
        || email.to_owned(), // If regex fails, return original
        |regex| {
            regex
                .replace_all(email, |caps: &regex::Captures| {
                    let full_match = &caps[0];
                    full_match.find('@').map_or_else(
                        || full_match.to_owned(),
                        |at_pos| {
                            let (local, domain_with_at) = full_match.split_at(at_pos);
                            let domain = &domain_with_at[1..]; // Skip '@'

                            let masked_local = if local.len() > 1 {
                                format!("{}***", &local[0..1])
                            } else {
                                local.to_owned()
                            };

                            let masked_domain = domain.find('.').map_or_else(
                                || domain.to_owned(),
                                |dot_pos| {
                                    let (subdomain, tld_with_dot) = domain.split_at(dot_pos);
                                    if subdomain.len() > 1 {
                                        format!("{}***{tld_with_dot}", &subdomain[0..1])
                                    } else {
                                        domain.to_owned()
                                    }
                                },
                            );

                            format!("{masked_local}@{masked_domain}")
                        },
                    )
                })
                .to_string()
        },
    )
}

/// Mask a recipient phone id for INFO+ logs — keep only the last 4 characters.
///
/// Used for messaging delivery-status logs (e.g. Meta `WhatsApp` `recipient_id`,
/// which is a phone number). Everything but the final 4 characters is replaced
/// with a run of `*`; ids of 4 characters or fewer are fully masked to `****`.
///
/// Slicing walks Unicode scalar values (`chars`), never byte indices, so a
/// recipient id containing a multibyte UTF-8 character never triggers a
/// "byte index is not a char boundary" panic.
///
/// # Examples
///
/// ```
/// use pierre_middleware::redaction::mask_recipient;
///
/// assert_eq!(mask_recipient("14502244753"), "*******4753");
/// assert_eq!(mask_recipient("123"), "****");
/// ```
#[must_use]
pub fn mask_recipient(id: &str) -> String {
    let char_count = id.chars().count();
    if char_count <= 4 {
        return "****".to_owned();
    }
    let last_four: String = id.chars().skip(char_count - 4).collect();
    format!("{}{last_four}", "*".repeat(char_count - 4))
}

/// Redact token-like patterns from text
///
/// Matches patterns like:
/// - Bearer `<token>`
/// - JWT `<token>`
/// - API key formats
///
/// # Arguments
///
/// * `text` - Text that may contain token patterns
/// * `config` - Redaction configuration
///
/// # Returns
///
/// Text with tokens redacted
#[must_use]
pub fn redact_token_patterns(text: &str, config: &RedactionConfig) -> String {
    if config.is_disabled() {
        return text.to_owned();
    }

    let mut result = text.to_owned();

    // Redact Bearer tokens
    if let Ok(re) = Regex::new(r"Bearer\s+[A-Za-z0-9\-._~+/]+=*") {
        result = re
            .replace_all(&result, format!("Bearer {}", config.redaction_placeholder))
            .to_string();
    }

    // Redact JWT-like tokens (three base64 segments separated by dots)
    if let Ok(re) = Regex::new(r"[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+\.[A-Za-z0-9\-_]+") {
        result = re
            .replace_all(&result, &config.redaction_placeholder)
            .to_string();
    }

    result
}

/// Bounded metric label for tenant IDs to prevent cardinality explosions
///
/// Limits the number of unique tenant IDs tracked in Prometheus metrics
/// by hashing tenant IDs into a fixed set of buckets.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedTenantLabel {
    bucket: String,
}

impl BoundedTenantLabel {
    /// Maximum number of tenant buckets for metrics (prevents unbounded cardinality)
    const MAX_BUCKETS: usize = 100;

    /// Create bounded label from tenant ID
    ///
    /// # Arguments
    ///
    /// * `tenant_id` - Raw tenant ID (UUID or string)
    ///
    /// # Returns
    ///
    /// Bounded label that hashes tenant to one of `MAX_BUCKETS` values
    #[must_use]
    pub fn new(tenant_id: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        tenant_id.hash(&mut hasher);
        let hash = hasher.finish();
        let bucket_id = hash % (Self::MAX_BUCKETS as u64);

        Self {
            bucket: format!("tenant_bucket_{bucket_id}"),
        }
    }

    /// Get the bucket label for metrics
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.bucket
    }
}

impl Display for BoundedTenantLabel {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.bucket)
    }
}

/// Bounded metric label for user IDs
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoundedUserLabel {
    bucket: String,
}

impl BoundedUserLabel {
    /// Maximum number of user buckets for metrics
    const MAX_BUCKETS: usize = 100;

    /// Create bounded label from user ID
    #[must_use]
    pub fn new(user_id: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        user_id.hash(&mut hasher);
        let hash = hasher.finish();
        let bucket_id = hash % (Self::MAX_BUCKETS as u64);

        Self {
            bucket: format!("user_bucket_{bucket_id}"),
        }
    }

    /// Get the bucket label for metrics
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.bucket
    }
}

impl Display for BoundedUserLabel {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.bucket)
    }
}

/// Get compiled email regex (cached)
///
/// Returns None if regex compilation fails (should never happen with hardcoded pattern)
fn email_regex() -> Option<&'static Regex> {
    static EMAIL_REGEX: OnceLock<Option<Regex>> = OnceLock::new();
    EMAIL_REGEX
        .get_or_init(|| {
            // Hardcoded regex pattern - should always compile
            Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").ok()
        })
        .as_ref()
}

/// Redact secret and PII values out of a URL query string.
///
/// Parameter *names* survive — an operator needs to know a callback carried a
/// `code` — while the value of any name in [`SENSITIVE_QUERY_PARAMS`] becomes
/// the configured placeholder. Every other value still passes through
/// [`mask_email`], so an address handed to an unlisted parameter is masked
/// rather than logged whole.
///
/// # Examples
///
/// ```
/// use pierre_middleware::redaction::{redact_query, RedactionConfig};
///
/// let config = RedactionConfig::default();
/// assert_eq!(
///     redact_query("code=abc123&provider=strava", &config),
///     "code=[REDACTED]&provider=strava"
/// );
/// ```
#[must_use]
pub fn redact_query(query: &str, config: &RedactionConfig) -> String {
    if config.is_disabled() {
        return query.to_owned();
    }

    query
        .split('&')
        .map(|pair| {
            let Some((name, value)) = pair.split_once('=') else {
                return mask_email(pair);
            };
            if SENSITIVE_QUERY_PARAMS.contains(&name.to_lowercase().as_str()) {
                format!("{name}={}", config.redaction_placeholder)
            } else {
                format!("{name}={}", mask_email(value))
            }
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the log-safe request line for a URI: path plus redacted query.
///
/// The path itself is kept verbatim — it is the route, and routes are what an
/// operator reads an alert for.
#[must_use]
pub fn redacted_request_line(uri: &Uri, config: &RedactionConfig) -> String {
    let path = uri.path();
    match uri.query() {
        Some(query) if !query.is_empty() => format!("{path}?{}", redact_query(query, config)),
        _ => path.to_owned(),
    }
}

/// The log-safe request line [`redaction_middleware`] attaches to every request.
///
/// Anything that logs the endpoint of a request reads this instead of
/// `Uri::to_string`, so a secret in the query never reaches a log sink.
#[derive(Debug, Clone)]
pub struct RedactedRequestLine(pub String);

impl Display for RedactedRequestLine {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Axum middleware that attaches a PII-redacted request line to each request.
///
/// Installed outside the failure logger and the tower-http `TraceLayer` so both
/// find [`RedactedRequestLine`] in the request extensions and log it in place of
/// the raw URI. Without it the `OAuth` callback query — authorization `code`,
/// `state` — and password-reset tokens land verbatim in `INFO`-level HTTP spans.
///
/// # Example
///
/// ```rust,no_run
/// use axum::{Router, routing::get, middleware};
/// use std::sync::Arc;
/// use pierre_middleware::redaction::{redaction_middleware, RedactionConfig};
///
/// # async fn handler() -> &'static str { "" }
/// let config = Arc::new(RedactionConfig::default());
/// let app: Router<()> = Router::new()
///     .route("/", get(handler))
///     .layer(middleware::from_fn_with_state(config, redaction_middleware));
/// ```
pub async fn redaction_middleware(
    State(config): State<Arc<RedactionConfig>>,
    mut req: Request,
    next: Next,
) -> Response {
    let line = redacted_request_line(req.uri(), &config);
    req.extensions_mut().insert(RedactedRequestLine(line));
    next.run(req).await
}
