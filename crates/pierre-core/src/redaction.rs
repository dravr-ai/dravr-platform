// ABOUTME: URL redaction utility for safe logging of connection strings
// ABOUTME: Strips credentials from URLs to prevent secret leakage in logs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Query-parameter name segments whose value is a credential.
///
/// A parameter is sensitive when any `_`/`-`/`.`-separated segment of its
/// lowercased name is one of these, so `api_key`, `access_token`,
/// `refresh_token`, `client_secret` and `X-Api-Key` all match while
/// `keyword` or `tokenizer` do not.
const SENSITIVE_QUERY_SEGMENTS: [&str; 6] =
    ["password", "passwd", "secret", "token", "key", "apikey"];

/// Redact credentials from a URL for safe logging
///
/// Replaces the `user:password` portion of a URL with `***:***` when
/// credentials are found between the scheme and an `@` symbol, and replaces
/// the value of every query parameter that names a credential (`password`,
/// `secret`, `token`, `key`, `api_key`, `access_token`, `refresh_token`,
/// `client_secret`, matched case-insensitively) with `***`. Every other part
/// of the URL is kept as written.
///
/// # Arguments
///
/// * `url` - URL that may contain embedded credentials
///
/// # Returns
///
/// URL with credentials redacted, showing only host and port
///
/// # Examples
///
/// ```
/// use pierre_core::redaction::redact_url;
///
/// // Redacts credentials from Redis URL
/// assert_eq!(
///     redact_url("redis://user:secret@localhost:6379"),
///     "redis://***:***@localhost:6379"
/// );
///
/// // Redacts a credential carried as a query parameter
/// assert_eq!(
///     redact_url("https://api.example.com/v1?api_key=abc123&page=2"),
///     "https://api.example.com/v1?api_key=***&page=2"
/// );
///
/// // Leaves URLs without credentials unchanged
/// assert_eq!(
///     redact_url("redis://localhost:6379"),
///     "redis://localhost:6379"
/// );
/// ```
#[must_use]
pub fn redact_url(url: &str) -> String {
    let (mut out, rest) = redact_userinfo(url);
    let (without_fragment, fragment) = rest.find('#').map_or((rest, ""), |at| rest.split_at(at));
    let (base, query) = without_fragment
        .split_once('?')
        .map_or((without_fragment, None), |(base, query)| {
            (base, Some(query))
        });
    out.push_str(base);
    if let Some(query) = query {
        out.push('?');
        out.push_str(&redact_query(query));
    }
    out.push_str(fragment);
    out
}

/// `url` split after its userinfo: the scheme and `***:***@` when the
/// authority carries userinfo, and the rest of the URL as written.
///
/// The authority runs to the first `/`, so an address in the path is never
/// mistaken for credentials. The userinfo is found before the query and the
/// fragment are split off, so a `?` or `#` written unencoded in a password
/// is redacted with it rather than leaking the password into the query.
fn redact_userinfo(url: &str) -> (String, &str) {
    if let Some(scheme_end) = url.find("://") {
        let authority_start = scheme_end + 3;
        let authority_end = url[authority_start..]
            .find('/')
            .map_or(url.len(), |at| authority_start + at);
        if let Some(at) = url[authority_start..authority_end].rfind('@') {
            return (
                format!("{}***:***@", &url[..authority_start]),
                &url[authority_start + at + 1..],
            );
        }
    }
    (String::new(), url)
}

/// `query` with the value of every credential-naming parameter replaced by `***`.
fn redact_query(query: &str) -> String {
    query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((name, _)) if is_sensitive_query_name(name) => format!("{name}=***"),
            _ => pair.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Whether a query-parameter name names a credential.
fn is_sensitive_query_name(name: &str) -> bool {
    name.to_ascii_lowercase()
        .split(['_', '-', '.'])
        .any(|segment| SENSITIVE_QUERY_SEGMENTS.contains(&segment))
}
