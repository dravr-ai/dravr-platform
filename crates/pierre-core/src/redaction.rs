// ABOUTME: URL redaction utility for safe logging of connection strings
// ABOUTME: Strips credentials from URLs to prevent secret leakage in logs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use percent_encoding::percent_decode_str;

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

/// Whether a query-parameter name names a credential. The name is compared
/// percent-decoded, so `api%5Fkey` is the `api_key` a server reads.
fn is_sensitive_query_name(name: &str) -> bool {
    percent_decode_str(name)
        .decode_utf8_lossy()
        .to_ascii_lowercase()
        .split(['_', '-', '.'])
        .any(|segment| SENSITIVE_QUERY_SEGMENTS.contains(&segment))
}

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;

    use super::redact_url;

    /// A secret no other generated part can contain, so finding it in the
    /// output can only mean it leaked.
    fn secret() -> impl Strategy<Value = String> {
        "[A-Za-z0-9]{12}".prop_map(|s| format!("XYZZY{s}"))
    }

    /// A userinfo password: RFC 3986 userinfo characters, plus `?` and `#`
    /// written unencoded, which the redactor promises to catch too.
    fn password() -> impl Strategy<Value = String> {
        (secret(), "[A-Za-z0-9._~!$&'()*+,;=:?#-]{0,8}").prop_map(|(s, tail)| format!("{s}{tail}"))
    }

    /// A query-parameter name the redactor must treat as a credential.
    fn credential_name() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("password"),
            Just("api_key"),
            Just("access_token"),
            Just("refresh_token"),
            Just("client_secret"),
            Just("X-Api-Key"),
            Just("apikey"),
            Just("auth.token"),
        ]
        .prop_flat_map(|name| {
            (vec(any::<bool>(), name.len()), any::<bool>()).prop_map(move |(upper, encoded)| {
                let name: String = name
                    .chars()
                    .zip(upper)
                    .map(|(c, up)| {
                        if up {
                            c.to_ascii_uppercase()
                        } else {
                            c.to_ascii_lowercase()
                        }
                    })
                    .collect();
                // A server decodes `%5F` to the `_` it stands for.
                if encoded {
                    name.replace('_', "%5F")
                } else {
                    name
                }
            })
        })
    }

    proptest! {
        #[test]
        fn any_input_is_handled_without_panicking(input in any::<String>()) {
            let _ = redact_url(&input);
        }

        #[test]
        fn a_userinfo_password_never_survives(
            scheme in "(redis|postgres|https|amqp)",
            user in "[a-z]{1,8}",
            pass in password(),
            host in "[a-z]{1,10}(\\.[a-z]{2,4})?(:[0-9]{2,5})?",
            path in "(/[a-z0-9]{0,8}){0,3}",
        ) {
            let url = format!("{scheme}://{user}:{pass}@{host}{path}");
            let out = redact_url(&url);
            prop_assert!(!out.contains("XYZZY"), "{url} -> {out}");
            prop_assert!(out.contains(&host), "the host is kept: {url} -> {out}");
        }

        #[test]
        fn a_credential_query_value_never_survives(
            name in credential_name(),
            value in secret(),
            before in "([a-z]{1,6}=[a-z0-9]{0,6}&){0,3}",
            after in "(&[a-z]{1,6}=[a-z0-9]{0,6}){0,3}",
            fragment in "(#[a-z]{0,6})?",
        ) {
            let url = format!("https://api.example.com/v1?{before}{name}={value}{after}{fragment}");
            let out = redact_url(&url);
            prop_assert!(!out.contains("XYZZY"), "{url} -> {out}");
        }

        #[test]
        fn a_url_without_credentials_is_unchanged(
            host in "[a-z]{1,10}\\.(com|io|dev)",
            path in "(/[a-z0-9]{1,8}){0,3}",
            query in "(\\?(page|limit|sort|q)=[a-z0-9]{0,6}(&(page|limit|sort|q)=[a-z0-9]{0,6}){0,2})?",
        ) {
            let url = format!("https://{host}{path}{query}");
            prop_assert_eq!(redact_url(&url), url);
        }

        #[test]
        fn redacting_twice_changes_nothing(input in any::<String>()) {
            let once = redact_url(&input);
            prop_assert_eq!(redact_url(&once), once);
        }
    }
}
