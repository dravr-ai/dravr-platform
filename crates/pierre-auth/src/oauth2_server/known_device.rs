// ABOUTME: The known-device cookie a browser earns by signing in: an HMAC-signed device id bound to one account
// ABOUTME: Lets that browser's next password sign-ins skip the account-wide window that strangers can fill (carnet#1088)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Known-device cookies (OWASP "device cookies").
//!
//! The account window meters every password sign-in naming one email, from
//! any address, so a stranger who knows an athlete's email can keep it full
//! and lock their sign-in out. A browser that signed in to that account
//! before carries a cookie proving it; its attempts are metered in a window of
//! their own instead, which a stranger cannot reach.
//!
//! The cookie is stateless: `v1.<expires>.<device id>.<signature>`, the
//! signature an HMAC-SHA256 over the account's email digest, the expiry and
//! the device id, with a key derived from the server's JWT secret. A cookie is
//! valid only for the email it was issued to, so signing in to one's own
//! account never yields a cookie for anyone else's.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::hmac;
use ring::rand::SystemRandom;
use uuid::Uuid;

use crate::security::cookies::{host_cookie_name, SameSitePolicy, SecureCookieConfig};

/// The cookie's name before the host prefix.
const KNOWN_DEVICE_COOKIE: &str = "known_device";

/// How long a browser stays known after its last successful sign-in.
const KNOWN_DEVICE_COOKIE_MAX_AGE_SECS: i64 = 180 * 24 * 60 * 60;

/// The format tag every cookie starts with, so a future format can be told apart.
const FORMAT: &str = "v1";

/// The label the cookie key is derived under, keeping it apart from every
/// other key derived from the same JWT secret.
const KEY_LABEL: &[u8] = b"pierre-known-device-signing-key-v1";

/// Signs and checks known-device cookies.
#[derive(Debug)]
pub struct KnownDeviceKey {
    key: hmac::Key,
}

impl KnownDeviceKey {
    /// The key every replica derives from the server's JWT secret, so a
    /// cookie one replica issued is honoured by all of them and survives a
    /// restart.
    #[must_use]
    pub fn from_jwt_secret(jwt_secret: &str) -> Self {
        let parent = hmac::Key::new(hmac::HMAC_SHA256, jwt_secret.as_bytes());
        let derived = hmac::sign(&parent, KEY_LABEL);
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, derived.as_ref()),
        }
    }

    /// A key of this process alone: its cookies stop being honoured when the
    /// process ends, and the sign-in falls back to the account window. For a
    /// limiter the composition root has not given the server's key.
    #[must_use]
    pub(crate) fn ephemeral() -> Self {
        let parent = hmac::Key::generate(hmac::HMAC_SHA256, &SystemRandom::new())
            .unwrap_or_else(|_| hmac::Key::new(hmac::HMAC_SHA256, Uuid::new_v4().as_bytes()));
        let derived = hmac::sign(&parent, KEY_LABEL);
        Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, derived.as_ref()),
        }
    }

    /// A fresh cookie value proving this browser signed in to the account
    /// whose email digest is `email_digest`.
    pub(crate) fn issue(&self, email_digest: &str) -> String {
        let expires = unix_now().saturating_add(KNOWN_DEVICE_COOKIE_MAX_AGE_SECS);
        let device = Uuid::new_v4().simple().to_string();
        let signature = self.sign(email_digest, expires, &device);
        format!("{FORMAT}.{expires}.{device}.{signature}")
    }

    /// The device id `value` carries, when it is a cookie this key issued for
    /// `email_digest` and it has not expired.
    pub(crate) fn device(&self, value: &str, email_digest: &str) -> Option<String> {
        let mut parts = value.split('.');
        let (format, expires, device, signature) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        if format != FORMAT || parts.next().is_some() {
            return None;
        }
        let expires_at: i64 = expires.parse().ok()?;
        if expires_at <= unix_now() || Uuid::try_parse(device).is_err() {
            return None;
        }
        let signature = URL_SAFE_NO_PAD.decode(signature).ok()?;
        hmac::verify(
            &self.key,
            signed_message(email_digest, expires_at, device).as_bytes(),
            &signature,
        )
        .ok()?;
        Some(device.to_owned())
    }

    /// The signature over the cookie's claims, base64url.
    fn sign(&self, email_digest: &str, expires: i64, device: &str) -> String {
        let tag = hmac::sign(
            &self.key,
            signed_message(email_digest, expires, device).as_bytes(),
        );
        URL_SAFE_NO_PAD.encode(tag.as_ref())
    }
}

/// The name the cookie is set and read under: `__Host-known_device` on an
/// HTTPS origin, so a sibling host cannot plant one.
#[must_use]
pub fn known_device_cookie_name(secure: bool) -> String {
    host_cookie_name(KNOWN_DEVICE_COOKIE, secure)
}

/// The `Set-Cookie` value that stores `value` in the browser: `HttpOnly`,
/// `SameSite=Lax`, for 180 days.
#[must_use]
pub fn known_device_set_cookie(value: String, secure: bool) -> String {
    SecureCookieConfig {
        name: known_device_cookie_name(secure),
        value,
        max_age_secs: KNOWN_DEVICE_COOKIE_MAX_AGE_SECS,
        http_only: true,
        secure,
        same_site: SameSitePolicy::Lax,
        path: "/".to_owned(),
    }
    .build()
}

/// What the signature covers.
fn signed_message(email_digest: &str, expires: i64, device: &str) -> String {
    format!("{FORMAT}|{email_digest}|{expires}|{device}")
}

/// Seconds since the Unix epoch.
fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ATHLETE: &str = "digest-of-athlete";

    #[test]
    fn a_cookie_names_its_device_for_the_account_it_was_issued_to() {
        let key = KnownDeviceKey::from_jwt_secret("secret");
        let cookie = key.issue(ATHLETE);
        let device = key.device(&cookie, ATHLETE);
        assert!(device.is_some());
        // Each sign-in issues a new device id
        let next = key.device(&key.issue(ATHLETE), ATHLETE);
        assert!(next.is_some());
        assert_ne!(device, next);
    }

    #[test]
    fn a_cookie_is_refused_for_another_account() {
        let key = KnownDeviceKey::from_jwt_secret("secret");
        let cookie = key.issue("digest-of-attacker");
        assert_eq!(key.device(&cookie, ATHLETE), None);
    }

    #[test]
    fn every_replica_honours_a_cookie_and_another_secret_does_not() {
        let cookie = KnownDeviceKey::from_jwt_secret("secret").issue(ATHLETE);
        assert!(KnownDeviceKey::from_jwt_secret("secret")
            .device(&cookie, ATHLETE)
            .is_some());
        assert_eq!(
            KnownDeviceKey::from_jwt_secret("other").device(&cookie, ATHLETE),
            None
        );
    }

    #[test]
    fn a_tampered_or_expired_cookie_is_refused() {
        let key = KnownDeviceKey::from_jwt_secret("secret");
        let cookie = key.issue(ATHLETE);
        let parts: Vec<&str> = cookie.split('.').collect();

        let other_device = Uuid::new_v4().simple().to_string();
        let swapped = format!("{}.{}.{other_device}.{}", parts[0], parts[1], parts[3]);
        assert_eq!(key.device(&swapped, ATHLETE), None);

        let later = parts[1].parse::<i64>().unwrap() + 1;
        let extended = format!("{}.{later}.{}.{}", parts[0], parts[2], parts[3]);
        assert_eq!(key.device(&extended, ATHLETE), None);

        let past = unix_now() - 1;
        let expired = format!(
            "{FORMAT}.{past}.{}.{}",
            parts[2],
            key.sign(ATHLETE, past, parts[2])
        );
        assert_eq!(key.device(&expired, ATHLETE), None);

        for junk in ["", "v1", "v2.1.2.3", &format!("{cookie}.extra")] {
            assert_eq!(key.device(junk, ATHLETE), None, "{junk:?}");
        }
    }

    #[test]
    fn the_cookie_is_host_only_and_http_only() {
        let set = known_device_set_cookie("value".to_owned(), true);
        assert!(set.starts_with("__Host-known_device=value;"), "{set}");
        assert!(set.contains("HttpOnly") && set.contains("Secure"), "{set}");
        assert!(
            set.contains("SameSite=Lax") && set.contains("Path=/"),
            "{set}"
        );
        assert!(known_device_set_cookie("v".to_owned(), false).starts_with("known_device=v;"));
    }
}
