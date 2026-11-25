// ABOUTME: The one emit site of provider.connect_failed: a provider sign-in that ended without connecting
// ABOUTME: Names the path (credential, browser, oauth) and the reason class; never the account, password or provider text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `provider.connect_failed`, the operational event every sign-in path
//! reports a failure through.
//!
//! On 2026-11-24 an athlete could not connect Strava in onboarding: Strava
//! turned the email sign-in down, and nothing but one log line said so. Three
//! athletes hit it in six days. A refusal now reaches the notify channel the
//! same way whichever path the athlete took, classed by why it ended, so a
//! provider turning everyone down reads differently from a sign-in service
//! that is down. The event carries ids and classes only: the account typed at
//! the provider and the provider's own words stay in the WARN log, masked.

use tracing::info;
use uuid::Uuid;

/// How the athlete was signing in to the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignInPath {
    /// The email and password sciotte signs in with (Google and Apple
    /// included), and the code steps that continue it.
    #[cfg(feature = "provider-sciotte")]
    Credential,
    /// The extension or the app's `WebView` handing over the provider session
    /// the athlete signed in to in their own browser.
    #[cfg(feature = "provider-sciotte")]
    Browser,
    /// The provider's own OAuth consent screen and its callback.
    OAuth,
}

impl SignInPath {
    /// The value the event carries.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            #[cfg(feature = "provider-sciotte")]
            Self::Credential => "credential",
            #[cfg(feature = "provider-sciotte")]
            Self::Browser => "browser",
            Self::OAuth => "oauth",
        }
    }
}

/// Why a sign-in ended without connecting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignInFailure {
    /// The provider refused the account, the session or the grant.
    TurnedDown,
    /// The sign-in could not run now: backpressure, or a sign-in that lapsed
    /// and has to start again.
    Unavailable,
    /// The provider stopped at a security check only the athlete can answer.
    #[cfg(feature = "provider-sciotte")]
    Challenge,
    /// The sign-in service or the platform failed.
    System,
}

impl SignInFailure {
    /// The value the event carries.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TurnedDown => "turned_down",
            Self::Unavailable => "unavailable",
            #[cfg(feature = "provider-sciotte")]
            Self::Challenge => "challenge",
            Self::System => "system",
        }
    }
}

/// Report a provider sign-in that ended without connecting.
///
/// `provider` is the backend name the connection would be stored under
/// (`sciotte`, `sciotte_garmin`, `strava`, …), as `sync.failed` names it.
/// `tenant_id` is `None` only for an OAuth callback whose state carried no
/// tenant.
pub(crate) fn report_connect_failed(
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    provider: &str,
    path: SignInPath,
    failure: SignInFailure,
) {
    let tenant_id = tenant_id.map(|tenant| tenant.to_string());
    info!(
        target: "notify",
        event = "provider.connect_failed",
        user_id = %user_id,
        tenant_id = tenant_id.as_deref(),
        provider = %provider,
        path = path.as_str(),
        reason_class = failure.as_str(),
        "provider sign-in failed"
    );
}

/// Report a failed credential sign-in, the one path `sciotte.rs` reports
/// from, for a caller that always knows its tenant.
#[cfg(feature = "provider-sciotte")]
pub(crate) fn report_credential_failed(
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    failure: SignInFailure,
) {
    report_connect_failed(
        user_id,
        Some(tenant_id),
        provider,
        SignInPath::Credential,
        failure,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // Wire values, pinned on purpose: the dravr-contremaitre catalogue of
    // `provider.connect_failed` lists them and the notify routing matches on
    // them. The sign-in service's own paths and class exist only in a build
    // that carries it, so they are pinned in their own test.
    #[test]
    fn the_event_values_are_the_catalogued_ones() {
        assert_eq!(
            SignInPath::OAuth.as_str(),
            "oauth",
            "bug: the OAuth path left the catalogued value"
        );
        let classes = [
            SignInFailure::TurnedDown,
            SignInFailure::Unavailable,
            SignInFailure::System,
        ]
        .map(SignInFailure::as_str);
        assert_eq!(
            classes,
            ["turned_down", "unavailable", "system"],
            "bug: a reason class left the catalogued value"
        );
    }

    #[cfg(feature = "provider-sciotte")]
    #[test]
    fn the_sign_in_service_values_are_the_catalogued_ones() {
        let paths = [SignInPath::Credential, SignInPath::Browser].map(SignInPath::as_str);
        assert_eq!(
            paths,
            ["credential", "browser"],
            "bug: a sign-in service path left the catalogued value"
        );
        assert_eq!(
            SignInFailure::Challenge.as_str(),
            "challenge",
            "bug: the challenge class left the catalogued value"
        );
    }
}
