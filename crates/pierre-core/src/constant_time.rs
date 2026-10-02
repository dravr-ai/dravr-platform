// ABOUTME: Constant-time equality for secrets, tokens and signatures, and the one blank-secret rule
// ABOUTME: The one comparison and the one configured-secret reader every webhook and OAuth verifier goes through
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;
use std::str;

use subtle::ConstantTimeEq;

/// Whether a configured secret is unset: empty, or nothing but whitespace
///
/// A variable set to nothing, or a secret-manager version holding a lone
/// newline, configures no secret. Keyed into an HMAC or compared against a
/// request, such a value is one anyone can reproduce, so every reader of a
/// configured secret answers it exactly as an absent one. This is that one
/// rule; [`configured_secret`] and [`matches_configured_secret`] apply it, and
/// a verifier holding a secret handed to it by a caller checks it here.
///
/// # Examples
///
/// ```
/// use pierre_core::constant_time::is_unset_secret;
///
/// assert!(is_unset_secret(b""));
/// assert!(is_unset_secret(b" \n\t"));
/// assert!(!is_unset_secret(b"whsec_abc"));
/// ```
#[must_use]
pub fn is_unset_secret(secret: &[u8]) -> bool {
    // Bytes that are all whitespace are valid UTF-8, so a value that is not
    // UTF-8 is never blank.
    str::from_utf8(secret).is_ok_and(|text| text.trim().is_empty())
}

/// The secret environment variable `var` configures, or `None` when it is
/// unset or blank ([`is_unset_secret`])
///
/// The value is returned as configured, untrimmed: surrounding whitespace in a
/// real secret is part of the key the other side signs with.
#[must_use]
pub fn configured_secret(var: &str) -> Option<String> {
    env::var(var)
        .ok()
        .filter(|secret| !is_unset_secret(secret.as_bytes()))
}

/// Whether two secrets are byte-for-byte equal, compared in constant time
///
/// `==` on strings or slices returns at the first differing byte, so the time
/// it takes tells a caller how long a prefix of their guess was right. This
/// reads every byte of equal-length inputs whatever their content.
///
/// Inputs of different lengths are unequal, and that answer comes without
/// looking at either one's content: the length of a secret is not hidden,
/// only its bytes.
///
/// Two empty inputs are equal. That is right only where both sides are derived
/// by the caller for this one exchange: a PKCE challenge against its verifier,
/// an ID-token nonce or an OAuth `state` against the one this server issued. A
/// verifier holding a secret from configuration calls
/// [`matches_configured_secret`] instead, which an unset secret cannot pass.
///
/// # Examples
///
/// ```
/// use pierre_core::constant_time::secrets_equal;
///
/// assert!(secrets_equal(b"whsec_abc", b"whsec_abc"));
/// assert!(!secrets_equal(b"whsec_abc", b"whsec_abd"));
/// assert!(!secrets_equal(b"whsec_abc", b"whsec_ab"));
/// ```
#[must_use]
pub fn secrets_equal(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Whether what a caller `presented` is the secret this deployment was
/// configured with, compared in constant time
///
/// `expected` is this side's value: a verify token or signing secret read from
/// the environment or a stored channel config, or a signature computed under
/// one. An unset `expected` ([`is_unset_secret`]: empty or whitespace only)
/// matches nothing, the same value presented back included: an unset secret
/// would otherwise be satisfied by a request that carries none, or carries the
/// blank it was set to. This is the one difference from [`secrets_equal`], and
/// the reason every verifier checking against configuration goes through here
/// instead of testing for emptiness itself.
///
/// A signature computed under an empty key is not empty, so a signature
/// verifier refuses its empty key before it signs; this function cannot see
/// the key.
///
/// # Examples
///
/// ```
/// use pierre_core::constant_time::matches_configured_secret;
///
/// assert!(matches_configured_secret(b"verify-me", b"verify-me"));
/// assert!(!matches_configured_secret(b"verify-me", b"forged"));
/// // Nothing was configured: nothing matches.
/// assert!(!matches_configured_secret(b"", b""));
/// // Set to a blank: still nothing configured.
/// assert!(!matches_configured_secret(b"\n", b"\n"));
/// ```
#[must_use]
pub fn matches_configured_secret(expected: &[u8], presented: &[u8]) -> bool {
    !is_unset_secret(expected) && secrets_equal(expected, presented)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::{configured_secret, is_unset_secret, matches_configured_secret, secrets_equal};

    #[test]
    fn equal_secrets_match() {
        assert!(secrets_equal(b"verify-me", b"verify-me"));
    }

    #[test]
    fn same_length_secrets_differing_anywhere_do_not_match() {
        assert!(!secrets_equal(b"verify-me", b"Xerify-me"));
        assert!(!secrets_equal(b"verify-me", b"veriXy-me"));
        assert!(!secrets_equal(b"verify-me", b"verify-mX"));
    }

    #[test]
    fn secrets_of_different_lengths_do_not_match() {
        assert!(!secrets_equal(b"verify-me", b"verify-me-too"));
        assert!(!secrets_equal(b"verify-me-too", b"verify-me"));
        // A shared prefix is not a match, in either direction.
        assert!(!secrets_equal(b"verify", b"verify-me"));
    }

    #[test]
    fn empty_matches_only_empty() {
        assert!(secrets_equal(b"", b""));
        assert!(!secrets_equal(b"", b"verify-me"));
        assert!(!secrets_equal(b"verify-me", b""));
    }

    #[test]
    fn a_configured_secret_matches_only_itself() {
        assert!(matches_configured_secret(b"verify-me", b"verify-me"));
        assert!(!matches_configured_secret(b"verify-me", b"verify-mX"));
        assert!(!matches_configured_secret(b"verify-me", b"verify"));
        assert!(!matches_configured_secret(b"verify-me", b""));
    }

    #[test]
    fn an_unset_secret_matches_nothing() {
        assert!(!matches_configured_secret(b"", b""));
        assert!(!matches_configured_secret(b"", b"verify-me"));
    }

    #[test]
    fn a_blank_secret_matches_nothing() {
        for blank in [&b" "[..], b"\n", b"\r\n", b" \t \n"] {
            assert!(!matches_configured_secret(blank, blank));
            assert!(!matches_configured_secret(blank, b""));
            assert!(!matches_configured_secret(blank, b"verify-me"));
        }
    }

    #[test]
    fn only_empty_or_whitespace_is_unset() {
        assert!(is_unset_secret(b""));
        assert!(is_unset_secret(b" \t\r\n"));
        assert!(!is_unset_secret(b"verify-me"));
        // Surrounding whitespace does not make a real secret unset.
        assert!(!is_unset_secret(b" verify-me\n"));
        // Bytes that are not text are a value, never a blank.
        assert!(!is_unset_secret(&[0xff, 0x20]));
    }

    #[test]
    fn a_blank_variable_configures_no_secret() {
        // A name no other test reads, so setting it races nothing.
        const VAR: &str = "PIERRE_CORE_CONSTANT_TIME_TEST_SECRET";
        env::remove_var(VAR);
        assert_eq!(configured_secret(VAR), None);
        for blank in ["", " ", "\n", " \t\r\n"] {
            env::set_var(VAR, blank);
            assert_eq!(configured_secret(VAR), None, "{blank:?} is unset");
        }
        env::set_var(VAR, " real-secret\n");
        assert_eq!(
            configured_secret(VAR).as_deref(),
            Some(" real-secret\n"),
            "a real secret comes back as configured, untrimmed"
        );
        env::remove_var(VAR);
    }
}
