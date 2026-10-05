// ABOUTME: The surface a call arrived through — one of Dravr's own apps, or a client outside them — and its class
// ABOUTME: A provider whose terms keep its data inside Dravr's own surfaces declares FirstPartyOnly; the rest AnyTransport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Which surface served a call (carnet#724).
//!
//! Some provider terms let Dravr show an athlete's data in its own web,
//! mobile and messaging surfaces but forbid making it available to anyone
//! else — through Dravr's API, an MCP server or an agent protocol — even in
//! derived form (Nolio API terms v1.0, §6.9). Each entry point declares the
//! [`Transport`] it serves; a provider declares its [`TransportPolicy`]; and
//! the provider-data filters ([`crate::ai_policy`]) drop a
//! [`TransportPolicy::FirstPartyOnly`] provider's items whenever the call is
//! external.
//!
//! The class is keyed on the entry point, never on the credential: the same
//! session token reaches `/mcp` and the web app's chat, and only the route
//! says which one served it.

use serde::{Deserialize, Serialize};

/// HTTP header Dravr's own clients set to name themselves (`"web"`,
/// `"mobile"`). Set by the client, so it labels a first-party session and
/// never classifies a call.
pub const CLIENT_PLATFORM_HEADER: &str = "x-client-platform";

/// The surface a call arrived through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// Dravr's web app, signed in with the athlete's session.
    WebApp,
    /// Dravr's mobile app, signed in with the athlete's session.
    MobileApp,
    /// A messaging channel the athlete linked (Telegram, `WhatsApp`, Discord,
    /// Slack, Messenger), including the turns it queues and resumes.
    Messaging,
    /// Work the platform runs for the athlete on its own, whose output only
    /// reaches the athlete's own surfaces.
    PlatformJob,
    /// An MCP client over HTTP (`POST /mcp`), whatever credential it presents.
    McpHttp,
    /// An agent speaking A2A.
    A2a,
    /// A caller holding one of the athlete's API keys, on the chat or REST
    /// routes.
    ApiKey,
}

impl Transport {
    /// Whether the call is served to one of Dravr's own surfaces.
    ///
    /// Only the variants listed here are first-party: a variant added later is
    /// external until it is listed, so forgetting to classify a new transport
    /// withholds data instead of leaking it.
    #[must_use]
    pub const fn is_first_party(self) -> bool {
        matches!(
            self,
            Self::WebApp | Self::MobileApp | Self::Messaging | Self::PlatformJob
        )
    }

    /// The app an athlete's session is signed in to, as the client's
    /// [`CLIENT_PLATFORM_HEADER`] names it: the mobile app when it says so,
    /// the web app otherwise. Both are first-party; the header only labels
    /// which.
    #[must_use]
    pub fn app_session(client_platform: Option<&str>) -> Self {
        if client_platform.is_some_and(|platform| platform.trim().eq_ignore_ascii_case("mobile")) {
            Self::MobileApp
        } else {
            Self::WebApp
        }
    }

    /// The transport of work started inside a call that already serves
    /// `self`: `inner` labels it, but an external call stays external — a
    /// declaration can only narrow where data may go, never widen it.
    #[must_use]
    pub const fn narrowed_by(self, inner: Self) -> Self {
        if self.is_first_party() {
            inner
        } else {
            self
        }
    }
}

/// Where a provider's terms let its data be served — and, stamped on content
/// derived from an athlete's data, where that content may be served
/// (carnet#769).
///
/// A derived row (a reply, a fact, a plan, a notification) is
/// [`Self::FirstPartyOnly`] when anything it was built from was: a
/// first-party-only provider's data, or another row so stamped. Rows written
/// before stamping existed read as [`Self::AnyTransport`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportPolicy {
    /// Only to Dravr's own surfaces: never over an external transport, in raw,
    /// derived or aggregated form.
    FirstPartyOnly,
    /// Over every transport. The default for a provider whose terms set no
    /// such restriction.
    #[default]
    AnyTransport,
}

impl TransportPolicy {
    /// The policy a stored `first_party_only` stamp column holds.
    #[must_use]
    pub const fn from_first_party_only(first_party_only: bool) -> Self {
        if first_party_only {
            Self::FirstPartyOnly
        } else {
            Self::AnyTransport
        }
    }

    /// Whether the content stays on Dravr's own surfaces — the value of a
    /// stored `first_party_only` stamp column.
    #[must_use]
    pub const fn is_first_party_only(self) -> bool {
        matches!(self, Self::FirstPartyOnly)
    }

    /// The stricter of two policies: content built from both sources may go
    /// only where both may.
    #[must_use]
    pub const fn strictest(self, other: Self) -> Self {
        Self::from_first_party_only(self.is_first_party_only() || other.is_first_party_only())
    }

    /// The strictest policy among `policies`; [`Self::AnyTransport`] when
    /// there are none.
    #[must_use]
    pub fn strictest_of(policies: impl IntoIterator<Item = Self>) -> Self {
        policies
            .into_iter()
            .fold(Self::AnyTransport, Self::strictest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, each with the class it must have. The match has no
    /// wildcard, so a new variant does not compile here until someone states
    /// its class — and `is_first_party` already makes it external.
    fn expected_first_party(transport: Transport) -> bool {
        match transport {
            Transport::WebApp
            | Transport::MobileApp
            | Transport::Messaging
            | Transport::PlatformJob => true,
            Transport::McpHttp | Transport::A2a | Transport::ApiKey => false,
        }
    }

    const ALL: [Transport; 7] = [
        Transport::WebApp,
        Transport::MobileApp,
        Transport::Messaging,
        Transport::PlatformJob,
        Transport::McpHttp,
        Transport::A2a,
        Transport::ApiKey,
    ];

    #[test]
    fn each_transport_has_its_class() {
        let external: Vec<Transport> = ALL.into_iter().filter(|t| !t.is_first_party()).collect();
        assert_eq!(
            external,
            vec![Transport::McpHttp, Transport::A2a, Transport::ApiKey]
        );
        for transport in ALL {
            assert_eq!(
                transport.is_first_party(),
                expected_first_party(transport),
                "{transport:?}"
            );
        }
    }

    #[test]
    fn a_session_is_labelled_by_its_client_and_always_first_party() {
        assert_eq!(Transport::app_session(Some("Mobile")), Transport::MobileApp);
        assert_eq!(Transport::app_session(Some("web")), Transport::WebApp);
        assert_eq!(Transport::app_session(None), Transport::WebApp);
        assert_eq!(
            Transport::app_session(Some("mcp")),
            Transport::WebApp,
            "a header claiming anything else is still only a session"
        );
    }

    #[test]
    fn a_stamp_is_as_strict_as_the_strictest_source() {
        use TransportPolicy::{AnyTransport, FirstPartyOnly};
        assert_eq!(AnyTransport.strictest(AnyTransport), AnyTransport);
        assert_eq!(AnyTransport.strictest(FirstPartyOnly), FirstPartyOnly);
        assert_eq!(FirstPartyOnly.strictest(AnyTransport), FirstPartyOnly);
        assert_eq!(TransportPolicy::strictest_of([]), AnyTransport);
        assert_eq!(
            TransportPolicy::strictest_of([AnyTransport, FirstPartyOnly, AnyTransport]),
            FirstPartyOnly
        );
        for policy in [AnyTransport, FirstPartyOnly] {
            assert_eq!(
                TransportPolicy::from_first_party_only(policy.is_first_party_only()),
                policy,
                "the stored column round-trips"
            );
        }
        assert_eq!(
            TransportPolicy::default(),
            AnyTransport,
            "an unstamped row is served everywhere"
        );
    }

    #[test]
    fn a_declaration_narrows_and_never_widens() {
        assert_eq!(
            Transport::WebApp.narrowed_by(Transport::Messaging),
            Transport::Messaging
        );
        assert_eq!(
            Transport::WebApp.narrowed_by(Transport::McpHttp),
            Transport::McpHttp
        );
        assert_eq!(
            Transport::McpHttp.narrowed_by(Transport::WebApp),
            Transport::McpHttp,
            "an external call stays external"
        );
        assert_eq!(
            Transport::ApiKey.narrowed_by(Transport::A2a),
            Transport::ApiKey
        );
    }
}
