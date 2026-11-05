// ABOUTME: OAuth Client ID Metadata Documents: a client_id that is an https URL naming its own metadata
// ABOUTME: Fetches the document through the SSRF guard, validates it, and keeps the result as a public client row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Client ID Metadata Documents (CIMD)
//!
//! A client whose `client_id` is an `https` URL needs no registration: the URL
//! serves a JSON document describing it, and the authorization server reads
//! that document the first time the `client_id` appears. Claude, `ChatGPT` and
//! Claude Code connect this way, which replaces one dynamic registration per
//! connection with one row per client.
//!
//! The URL is chosen by whoever sends the authorization request, so the fetch
//! is held to every rule that keeps it from reaching somewhere it should not:
//!
//! - `https` only, on the default port, with no credentials in the URL;
//! - no local-only name and no address in the tronc deny list (loopback,
//!   private, link-local, metadata, CGNAT, ...), checked on a literal address
//!   and on every address the name resolves to, at connect time, so a name
//!   rebound after a check is refused too;
//! - redirects are never followed: a 3xx is refused, so a document cannot
//!   bounce the fetch to an internal address;
//! - the body is capped at `MAX_DOCUMENT_BYTES`, the whole fetch at
//!   `FETCH_TIMEOUT`;
//! - the document's `client_id` must equal the URL it was fetched from.
//!
//! A document that passes becomes a public client row (no secret, so no secret
//! can ever authenticate it) whose `client_id` is the URL. The row is what the
//! authorization codes and refresh tokens point at, and it is rewritten from the
//! document again once `CACHE_TTL` has passed.

use super::client_registration::{
    ClientRegistrationManager, INACTIVITY_EXPIRY_DAYS, PUBLIC_CLIENT_SECRET_HASH,
};
use super::models::{OAuth2Client, OAuth2Error};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use dravr_tronc::http_client::guard::{
    url_digest, FetchLimits, FetchPolicy, GuardError, GuardedClient,
};
use pierre_core::errors::AppError;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_database::backends::OAuth2ServerRepository;
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration as StdDuration, Instant};
use tracing::{info, warn};
use url::Url;
use uuid::Uuid;

/// The largest metadata document read. Real ones are a few hundred bytes.
const MAX_DOCUMENT_BYTES: u64 = 64 * 1024;

/// The longest the whole fetch may take, connect and body included.
const FETCH_TIMEOUT: StdDuration = StdDuration::from_secs(5);

/// How long a fetched document is used before it is fetched again.
const CACHE_TTL: StdDuration = StdDuration::from_hours(24);

/// The most redirect URIs a document may list.
const MAX_REDIRECT_URIS: usize = 20;

/// The longest `client_id` URL accepted.
const MAX_CLIENT_ID_LEN: usize = 2048;

/// The longest display name kept.
const MAX_CLIENT_NAME_LEN: usize = 200;

/// Whether `client_id` has the shape of a metadata document URL. It says
/// nothing about whether the URL may be fetched.
#[must_use]
pub fn is_metadata_document_id(client_id: &str) -> bool {
    client_id.starts_with("https://")
}

/// Why a client metadata document could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CimdError {
    /// The `client_id` is not an acceptable document URL.
    InvalidUrl(&'static str),
    /// The guard refused the fetch or it failed in transport.
    Fetch(GuardError),
    /// The document server answered something other than 200.
    Status(u16),
    /// The body is not a JSON object of the expected shape.
    NotADocument,
    /// The document's `client_id` is not the URL it was fetched from.
    ClientIdMismatch,
    /// The document breaks a rule of the metadata.
    Invalid(String),
    /// Storing the client failed, or the registration ceiling is reached.
    Store(String),
}

impl fmt::Display for CimdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(why) => write!(f, "client_id is not a usable metadata URL: {why}"),
            Self::Fetch(error) => write!(f, "client metadata document not fetched: {error}"),
            Self::Status(code) => {
                write!(f, "client metadata document answered HTTP {code}")
            }
            Self::NotADocument => f.write_str("client metadata document is not a JSON object"),
            Self::ClientIdMismatch => {
                f.write_str("client metadata document client_id does not equal its URL")
            }
            Self::Invalid(why) => write!(f, "client metadata document is invalid: {why}"),
            Self::Store(why) => write!(f, "client metadata document could not be stored: {why}"),
        }
    }
}

impl Error for CimdError {}

impl CimdError {
    /// The `OAuth2` error an authorization request is refused with: the
    /// caller's mistake as `invalid_client`, and a server fault as such.
    #[must_use]
    pub fn into_oauth2_error(self) -> OAuth2Error {
        if self.is_transient() {
            return OAuth2Error::temporarily_unavailable(
                "The client metadata document could not be fetched",
            );
        }
        match self {
            Self::Store(_) => OAuth2Error::server_error("The client could not be registered"),
            _ => OAuth2Error::invalid_client(),
        }
    }

    /// Whether the document's host may simply be down, as opposed to having
    /// answered that the document is gone or having served one that is refused.
    #[must_use]
    pub const fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Fetch(GuardError::Transport(_) | GuardError::TotalTimeout)
                | Self::Status(500..=599)
        )
    }
}

/// Reads the bytes a metadata URL serves.
#[async_trait]
pub trait MetadataFetcher: Send + Sync {
    /// Fetch `url` and return the response body of a 200.
    ///
    /// # Errors
    /// Returns the reason the document was not read; never a part of the body.
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, CimdError>;
}

/// The production fetcher: a tronc [`GuardedClient`] under the `webhook` policy.
///
/// The policy denies private addresses at connect time and follows no
/// redirects; this narrows it to `https` on the default port, with a small body
/// cap and a short timeout.
struct GuardedFetcher {
    client: GuardedClient,
    enforce_public_https: bool,
}

impl fmt::Debug for GuardedFetcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardedFetcher").finish_non_exhaustive()
    }
}

impl GuardedFetcher {
    /// The caps every fetch is held to.
    #[must_use]
    const fn limits() -> FetchLimits {
        FetchLimits {
            max_body_bytes: MAX_DOCUMENT_BYTES,
            connect_timeout: StdDuration::from_secs(3),
            idle_timeout: FETCH_TIMEOUT,
            total_timeout: FETCH_TIMEOUT,
        }
    }

    /// Build the production fetcher.
    ///
    /// # Errors
    /// Returns the guard's error when the TLS backend cannot be initialised.
    fn try_new() -> Result<Self, CimdError> {
        let client = GuardedClient::builder(FetchPolicy::webhook())
            .limits(Self::limits())
            .build()
            .map_err(CimdError::Fetch)?;
        Ok(Self {
            client,
            enforce_public_https: true,
        })
    }

    /// Wrap a client built elsewhere. `enforce_public_https: false` lifts the
    /// scheme and port rule, for a test serving the document from a local
    /// listener.
    #[cfg(test)]
    fn with_client(client: GuardedClient, enforce_public_https: bool) -> Self {
        Self {
            client,
            enforce_public_https,
        }
    }
}

#[async_trait]
impl MetadataFetcher for GuardedFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, CimdError> {
        if self.enforce_public_https {
            check_document_url(url)?;
        }
        let response = self
            .client
            .get(url)
            .map_err(CimdError::Fetch)?
            .send()
            .await
            .map_err(CimdError::Fetch)?;
        let status = response.status();
        if status.as_u16() != 200 {
            // Redirects are not followed: a 3xx lands here and is refused.
            return Err(CimdError::Status(status.as_u16()));
        }
        response.bytes().await.map_err(CimdError::Fetch)
    }
}

/// The rules on a document URL that need no network: an `https` URL on the
/// default port with a path, no credentials, no fragment, no dot segments,
/// spelled exactly as the parser prints it.
///
/// # Errors
/// Returns the first rule the URL breaks.
fn check_document_url(url: &str) -> Result<Url, CimdError> {
    if url.len() > MAX_CLIENT_ID_LEN {
        return Err(CimdError::InvalidUrl("too long"));
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(CimdError::InvalidUrl("whitespace or control character"));
    }
    let parsed = Url::parse(url).map_err(|_| CimdError::InvalidUrl("not a URL"))?;
    if parsed.scheme() != "https" {
        return Err(CimdError::InvalidUrl("not https"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(CimdError::InvalidUrl("credentials in the URL"));
    }
    if parsed.port().is_some() {
        return Err(CimdError::InvalidUrl("custom port"));
    }
    if parsed.fragment().is_some() {
        return Err(CimdError::InvalidUrl("fragment"));
    }
    if parsed.path().len() < 2 {
        return Err(CimdError::InvalidUrl("no path"));
    }
    if parsed
        .path_segments()
        .is_some_and(|mut segments| segments.any(|s| s == "." || s == ".."))
    {
        return Err(CimdError::InvalidUrl("dot segment"));
    }
    // A client_id that is not the parser's own spelling of its URL could name
    // one document to the registrar and another to the fetch.
    if parsed.as_str() != url {
        return Err(CimdError::InvalidUrl("not in canonical form"));
    }
    FetchPolicy::webhook()
        .check_url(url)
        .map_err(CimdError::Fetch)?;
    Ok(parsed)
}

/// The fields of a client metadata document the server reads.
#[derive(Debug, Deserialize)]
struct MetadataDocument {
    client_id: String,
    client_name: Option<String>,
    client_uri: Option<String>,
    redirect_uris: Vec<String>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
    #[serde(rename = "token_endpoint_auth_method")]
    auth_method: Option<String>,
    scope: Option<String>,
}

/// Validate fetched `body` as the document `url` serves, and build the public
/// client it describes, expiring `INACTIVITY_EXPIRY_DAYS` after `now`.
///
/// # Errors
/// Returns the rule the document breaks.
pub fn client_from_document(
    url: &str,
    body: &[u8],
    now: DateTime<Utc>,
) -> Result<OAuth2Client, CimdError> {
    let document: MetadataDocument =
        serde_json::from_slice(body).map_err(|_| CimdError::NotADocument)?;
    if document.client_id != url {
        return Err(CimdError::ClientIdMismatch);
    }
    // A URL names no secret anyone could keep: only `none` is a method a
    // metadata document may use.
    if document
        .auth_method
        .as_deref()
        .is_some_and(|method| method != "none")
    {
        return Err(CimdError::Invalid(
            "token_endpoint_auth_method must be none".to_owned(),
        ));
    }
    check_redirect_uris(&document.redirect_uris)?;

    let grant_types = document
        .grant_types
        .unwrap_or_else(|| vec!["authorization_code".to_owned()]);
    if grant_types
        .iter()
        .any(|g| !matches!(g.as_str(), "authorization_code" | "refresh_token"))
    {
        return Err(CimdError::Invalid("unsupported grant_types".to_owned()));
    }
    let response_types = document
        .response_types
        .unwrap_or_else(|| vec!["code".to_owned()]);
    if response_types.iter().any(|r| r != "code") {
        return Err(CimdError::Invalid("unsupported response_types".to_owned()));
    }
    let named = [
        document.client_name.as_deref(),
        document.client_uri.as_deref(),
    ];
    if named
        .into_iter()
        .flatten()
        .any(|v| v.chars().any(char::is_control))
    {
        return Err(CimdError::Invalid(
            "client_name and client_uri must not contain control characters".to_owned(),
        ));
    }
    let client_name = document
        .client_name
        .map(|name| name.chars().take(MAX_CLIENT_NAME_LEN).collect::<String>());

    // The ceiling a client that names no scope gets is every scope a third
    // party can be delegated; what each authorization grants is still what it
    // asks for, and the consent screen shows it.
    let scope = match document.scope.as_deref().map(str::trim) {
        Some(requested) if !requested.is_empty() => OAuthScope::requested_grant(Some(requested))
            .map_err(|e| CimdError::Invalid(e.message))?,
        _ => OAuthScope::delegable_grant(),
    };

    Ok(OAuth2Client {
        id: Uuid::new_v4().to_string(),
        client_id: url.to_owned(),
        client_secret_hash: PUBLIC_CLIENT_SECRET_HASH.to_owned(),
        redirect_uris: document.redirect_uris,
        grant_types,
        response_types,
        client_name,
        client_uri: document.client_uri,
        scope: Some(OAuthScope::render_granted(&scope)),
        created_at: now,
        expires_at: Some(now + Duration::days(INACTIVITY_EXPIRY_DAYS)),
    })
}

/// Every listed redirect URI must be acceptable to a registration: `https`, or
/// `http` on a loopback host.
fn check_redirect_uris(uris: &[String]) -> Result<(), CimdError> {
    if uris.is_empty() {
        return Err(CimdError::Invalid("redirect_uris is empty".to_owned()));
    }
    if uris.len() > MAX_REDIRECT_URIS {
        return Err(CimdError::Invalid("too many redirect_uris".to_owned()));
    }
    if uris
        .iter()
        .any(|uri| !ClientRegistrationManager::is_valid_redirect_uri(uri))
    {
        return Err(CimdError::Invalid(
            "a redirect_uri is not allowed".to_owned(),
        ));
    }
    Ok(())
}

/// When each document was last fetched by this process. The stored row is the
/// durable copy; this only decides whether it is old enough to fetch again.
static FETCHED: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The most documents the freshness map remembers; past it the oldest go.
const MAX_REMEMBERED: usize = 4096;

fn is_fresh(client_id: &str) -> bool {
    FETCHED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(client_id)
        .is_some_and(|fetched| fetched.elapsed() < CACHE_TTL)
}

fn remember(client_id: &str) {
    let mut fetched = FETCHED.lock().unwrap_or_else(PoisonError::into_inner);
    if fetched.len() >= MAX_REMEMBERED {
        fetched.retain(|_, at| at.elapsed() < CACHE_TTL);
    }
    if fetched.len() >= MAX_REMEMBERED {
        fetched.clear();
    }
    fetched.insert(client_id.to_owned(), Instant::now());
}

/// A fetcher that refuses everything, standing in when the guarded client
/// could not be built (its TLS backend failed to initialise).
struct UnavailableFetcher(GuardError);

#[async_trait]
impl MetadataFetcher for UnavailableFetcher {
    async fn fetch(&self, _url: &str) -> Result<Vec<u8>, CimdError> {
        Err(CimdError::Fetch(self.0.clone()))
    }
}

/// The process-wide guarded fetcher, built once.
static PRODUCTION_FETCHER: LazyLock<Arc<dyn MetadataFetcher>> =
    LazyLock::new(|| match GuardedFetcher::try_new() {
        Ok(fetcher) => Arc::new(fetcher),
        Err(CimdError::Fetch(error)) => Arc::new(UnavailableFetcher(error)),
        Err(other) => Arc::new(UnavailableFetcher(GuardError::Transport(other.to_string()))),
    });

/// The fetcher production uses: the SSRF-guarded one.
#[must_use]
pub(super) fn production_fetcher() -> Arc<dyn MetadataFetcher> {
    Arc::clone(&PRODUCTION_FETCHER)
}

/// What [`MetadataResolver::resolve`] found.
#[derive(Debug, Clone)]
pub(super) struct ResolvedClient {
    /// The client the document describes.
    pub(super) client: OAuth2Client,
    /// Whether it has a row yet; one without waits for an athlete to sign in.
    pub(super) is_stored: bool,
}

/// Resolves a metadata document `client_id` to the public client it describes.
pub(super) struct MetadataResolver {
    /// Shared with the authorization server, which builds a resolver per request
    fetcher: Arc<dyn MetadataFetcher>,
    /// Shared with the authorization server, which holds the one repository
    oauth2: Arc<dyn OAuth2ServerRepository>,
    max_pending_registrations: u64,
}

impl fmt::Debug for MetadataResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetadataResolver").finish_non_exhaustive()
    }
}

impl MetadataResolver {
    /// A resolver that stores through `oauth2`, fetching through `fetcher`.
    /// New rows count against the same pending-registration ceiling dynamic
    /// registration does.
    #[must_use]
    pub(super) fn new(
        fetcher: Arc<dyn MetadataFetcher>,
        oauth2: Arc<dyn OAuth2ServerRepository>,
        max_pending_registrations: u64,
    ) -> Self {
        Self {
            fetcher,
            oauth2,
            max_pending_registrations,
        }
    }

    /// The public client `client_id` names: the stored row while it is fresh,
    /// otherwise the document fetched again.
    ///
    /// This runs before anyone has signed in, so a client not seen before is
    /// fetched and checked but not stored: anyone can send an authorize
    /// request, and a row per request would fill the registration ceiling every
    /// real connection shares. [`Self::enroll`] stores it once an athlete is
    /// signed in. A stored row is refreshed from the document; when the
    /// document's host is down the row keeps serving, and a document that is
    /// gone or no longer valid is refused.
    ///
    /// # Errors
    /// Returns the reason the document was refused or could not be stored.
    pub(super) async fn resolve(&self, client_id: &str) -> Result<ResolvedClient, CimdError> {
        check_document_url(client_id)?;
        let stored = self
            .oauth2
            .get_client(client_id)
            .await
            .map_err(|e| CimdError::Store(e.to_string()))?;
        if let Some(stored) = &stored {
            if stored.client_secret_hash != PUBLIC_CLIENT_SECRET_HASH {
                // Never rewrite a confidential registration from a URL.
                return Err(CimdError::Invalid(
                    "client_id belongs to a registered client".to_owned(),
                ));
            }
            if is_fresh(client_id) {
                return Ok(ResolvedClient {
                    client: stored.clone(),
                    is_stored: true,
                });
            }
        }

        let digest = url_digest(client_id);
        let body = match (self.fetcher.fetch(client_id).await, stored) {
            (Ok(body), _) => body,
            (Err(error), Some(stored)) if error.is_transient() => {
                warn!(url_digest = %digest, error = %error, "client metadata document unreachable; using the stored client");
                return Ok(ResolvedClient {
                    client: stored,
                    is_stored: true,
                });
            }
            (Err(error), _) => {
                warn!(url_digest = %digest, error = %error, "client metadata document refused");
                return Err(error);
            }
        };
        self.refresh_from_document(client_id, &body, &digest).await
    }

    /// The client `body` describes, written over its row when it has one.
    async fn refresh_from_document(
        &self,
        client_id: &str,
        body: &[u8],
        digest: &str,
    ) -> Result<ResolvedClient, CimdError> {
        let client = client_from_document(client_id, body, Utc::now()).inspect_err(|error| {
            warn!(url_digest = %digest, error = %error, "client metadata document invalid");
        })?;
        let is_stored = self
            .oauth2
            .update_public_client_metadata(&client)
            .await
            .map_err(|e| CimdError::Store(e.to_string()))?;
        if is_stored {
            remember(client_id);
            info!(url_digest = %digest, "client metadata document refreshed");
        }
        Ok(ResolvedClient { client, is_stored })
    }

    /// Store a client [`Self::resolve`] read but did not store, counted against
    /// the pending-registration ceiling. Reached only once an athlete is signed
    /// in.
    ///
    /// # Errors
    /// Returns [`CimdError::Store`] when the ceiling is reached or the row
    /// cannot be written.
    pub(super) async fn enroll(&self, client: &OAuth2Client) -> Result<(), CimdError> {
        let store_error = |e: AppError| CimdError::Store(e.to_string());
        let stored = self
            .oauth2
            .store_client_within_ceiling(client, self.max_pending_registrations)
            .await;
        match stored {
            Ok(true) => {}
            Ok(false) => {
                return Err(CimdError::Store(
                    "too many clients are awaiting authorization".to_owned(),
                ));
            }
            Err(error) => {
                // A second sign-in through the same new client got there first.
                let present = self
                    .oauth2
                    .get_client(&client.client_id)
                    .await
                    .map_err(store_error)?;
                if present.is_none() {
                    return Err(store_error(error));
                }
            }
        }
        remember(&client.client_id);
        info!(url_digest = %url_digest(&client.client_id), "client metadata document stored");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dravr_tronc::http_client::guard::HostResolver;
    use std::future::Future;
    use std::io;
    use std::net::IpAddr;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::time;

    const URL: &str = "https://claude.ai/oauth/client.json";

    fn document(client_id: &str, extra: &str) -> Vec<u8> {
        format!(
            r#"{{"client_id":"{client_id}","client_name":"Claude","redirect_uris":["https://claude.ai/api/mcp/auth_callback"]{extra}}}"#
        )
        .into_bytes()
    }

    fn built(body: &[u8]) -> Result<OAuth2Client, CimdError> {
        client_from_document(URL, body, Utc::now())
    }

    #[test]
    fn a_valid_document_becomes_a_public_client_with_every_delegable_scope() {
        let client = built(&document(URL, "")).unwrap();
        assert_eq!(client.client_id, URL);
        assert_eq!(client.client_secret_hash, PUBLIC_CLIENT_SECRET_HASH);
        assert_eq!(client.grant_types, ["authorization_code"]);
        assert_eq!(client.response_types, ["code"]);
        let ceiling = OAuthScope::parse_granted(client.scope.as_deref().unwrap());
        assert_eq!(ceiling, OAuthScope::delegable_grant());
        let expires = client.expires_at.unwrap() - client.created_at;
        assert_eq!(expires, Duration::days(INACTIVITY_EXPIRY_DAYS));
    }

    #[test]
    fn a_document_scope_narrows_the_ceiling() {
        let client = built(&document(URL, r#","scope":"fitness:read""#)).unwrap();
        assert_eq!(client.scope.as_deref(), Some("fitness:read"));
    }

    #[test]
    fn a_document_naming_another_client_id_is_refused() {
        let body = document("https://evil.example/client.json", "");
        assert_eq!(built(&body).unwrap_err(), CimdError::ClientIdMismatch);
    }

    #[test]
    fn a_secret_bearing_auth_method_is_refused() {
        for method in [
            "client_secret_basic",
            "client_secret_post",
            "private_key_jwt",
        ] {
            let body = document(URL, &format!(r#","token_endpoint_auth_method":"{method}""#));
            assert!(
                matches!(built(&body), Err(CimdError::Invalid(_))),
                "{method}"
            );
        }
        let none = document(URL, r#","token_endpoint_auth_method":"none""#);
        assert!(built(&none).is_ok());
    }

    #[test]
    fn unusable_redirect_uris_are_refused() {
        let bad = [
            r#"["http://evil.example/cb"]"#,
            r#"["urn:ietf:wg:oauth:2.0:oob"]"#,
            r#"["https://a.example/cb#frag"]"#,
            r#"["https://*.example/cb"]"#,
            r"[]",
        ];
        for uris in bad {
            let body = format!(r#"{{"client_id":"{URL}","redirect_uris":{uris}}}"#);
            assert!(
                matches!(built(body.as_bytes()), Err(CimdError::Invalid(_))),
                "{uris}"
            );
        }
        let loopback = format!(
            r#"{{"client_id":"{URL}","redirect_uris":["http://localhost:1234/cb","http://127.0.0.1/cb"]}}"#
        );
        assert!(built(loopback.as_bytes()).is_ok());
    }

    #[test]
    fn a_body_that_is_not_a_document_is_refused() {
        assert_eq!(built(b"<html>").unwrap_err(), CimdError::NotADocument);
        assert_eq!(built(b"[1,2]").unwrap_err(), CimdError::NotADocument);
        assert_eq!(built(b"{}").unwrap_err(), CimdError::NotADocument);
    }

    #[test]
    fn unsupported_grant_and_response_types_are_refused() {
        for extra in [
            r#","grant_types":["client_credentials"]"#,
            r#","response_types":["token"]"#,
        ] {
            assert!(matches!(
                built(&document(URL, extra)),
                Err(CimdError::Invalid(_))
            ));
        }
    }

    #[test]
    fn a_url_that_could_not_be_fetched_safely_is_refused_before_any_network() {
        let refused = [
            "http://claude.ai/oauth/client.json",
            "https://claude.ai:8443/oauth/client.json",
            "https://user:pw@claude.ai/oauth/client.json",
            "https://claude.ai/oauth/client.json#frag",
            "https://claude.ai",
            "https://claude.ai/",
            "https://claude.ai/a/../client.json",
            "https://localhost/client.json",
            "https://metadata.google.internal/client.json",
            "https://printer.local/client.json",
            "https://127.0.0.1/client.json",
            "https://10.0.0.5/client.json",
            "https://192.168.1.1/client.json",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/client.json",
            "https://[fd00::1]/client.json",
            "https://0.0.0.0/client.json",
            "ftp://claude.ai/client.json",
            "not a url",
        ];
        for url in refused {
            assert!(check_document_url(url).is_err(), "{url} must be refused");
        }
        assert!(check_document_url(URL).is_ok());
    }

    /// Answers every lookup with fixed addresses, to stand in for DNS.
    struct FixedResolver(Vec<IpAddr>);

    impl HostResolver for FixedResolver {
        fn lookup(
            &self,
            _host: &str,
        ) -> Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>> {
            let addresses = self.0.clone();
            Box::pin(async move { Ok(addresses) })
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_a_private_address_is_refused_at_connect() {
        for address in [
            "10.1.2.3",
            "127.0.0.1",
            "169.254.169.254",
            "192.168.0.9",
            "::1",
        ] {
            let client = GuardedClient::builder(FetchPolicy::webhook())
                .limits(GuardedFetcher::limits())
                .resolver(Arc::new(FixedResolver(vec![address.parse().unwrap()])))
                .build()
                .unwrap();
            let fetcher = GuardedFetcher::with_client(client, true);
            let outcome = fetcher.fetch(URL).await;
            assert_eq!(
                outcome,
                Err(CimdError::Fetch(GuardError::DeniedAddress)),
                "{address}"
            );
        }
    }

    #[tokio::test]
    async fn one_private_address_among_public_ones_refuses_the_whole_answer() {
        let mixed: Vec<IpAddr> = vec![
            "93.184.216.34".parse().unwrap(),
            "10.0.0.1".parse().unwrap(),
        ];
        let client = GuardedClient::builder(FetchPolicy::webhook())
            .limits(GuardedFetcher::limits())
            .resolver(Arc::new(FixedResolver(mixed)))
            .build()
            .unwrap();
        let fetcher = GuardedFetcher::with_client(client, true);
        assert_eq!(
            fetcher.fetch(URL).await,
            Err(CimdError::Fetch(GuardError::DeniedAddress))
        );
    }

    /// Serve one canned response per connection on a loopback port; returns the
    /// URL and the number of connections served.
    async fn serve(response: Vec<u8>, stall: bool) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                counted.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buffer = [0u8; 2048];
                    if socket.read(&mut buffer).await.is_err() {
                        return;
                    }
                    if stall {
                        time::sleep(StdDuration::from_secs(30)).await;
                        return;
                    }
                    if socket.write_all(&response).await.is_ok() {
                        socket.shutdown().await.ok();
                    }
                });
            }
        });
        (format!("http://127.0.0.1:{port}/client.json"), hits)
    }

    fn loopback_fetcher(limits: FetchLimits) -> GuardedFetcher {
        let client = GuardedClient::builder(FetchPolicy::webhook())
            .limits(limits)
            .permit_loopback_for_tests()
            .build()
            .unwrap();
        GuardedFetcher::with_client(client, false)
    }

    fn http_ok(body: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        out.extend_from_slice(body);
        out
    }

    #[tokio::test]
    async fn a_200_body_is_returned() {
        let (url, _) = serve(http_ok(b"{\"ok\":true}"), false).await;
        let body = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert_eq!(body.unwrap(), b"{\"ok\":true}");
    }

    #[tokio::test]
    async fn a_redirect_is_refused_and_never_followed() {
        let (target, target_hits) = serve(http_ok(b"{}"), false).await;
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nlocation: {target}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        );
        let (url, hits) = serve(redirect.into_bytes(), false).await;
        let outcome = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert_eq!(outcome, Err(CimdError::Status(302)));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            target_hits.load(Ordering::SeqCst),
            0,
            "the redirect was followed"
        );
    }

    #[tokio::test]
    async fn a_redirect_to_a_metadata_address_is_not_followed() {
        let redirect = "HTTP/1.1 302 Found\r\nlocation: http://169.254.169.254/latest/meta-data\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (url, _) = serve(redirect.as_bytes().to_vec(), false).await;
        let outcome = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert_eq!(outcome, Err(CimdError::Status(302)));
    }

    #[tokio::test]
    async fn a_non_200_status_is_refused() {
        let (url, _) = serve(
            b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_vec(),
            false,
        )
        .await;
        let outcome = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert_eq!(outcome, Err(CimdError::Status(404)));
    }

    #[tokio::test]
    async fn a_body_past_the_size_cap_is_refused() {
        let big = vec![b'a'; 4096];
        let (url, _) = serve(http_ok(&big), false).await;
        let mut limits = GuardedFetcher::limits();
        limits.max_body_bytes = 1024;
        let outcome = loopback_fetcher(limits).fetch(&url).await;
        assert!(
            matches!(
                outcome,
                Err(CimdError::Fetch(GuardError::BodyTooLarge { .. }))
            ),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn a_server_that_stalls_is_cut_off_by_the_timeout() {
        let (url, _) = serve(Vec::new(), true).await;
        let mut limits = GuardedFetcher::limits();
        limits.idle_timeout = StdDuration::from_millis(300);
        limits.total_timeout = StdDuration::from_millis(300);
        let started = Instant::now();
        let outcome = loopback_fetcher(limits).fetch(&url).await;
        assert!(outcome.is_err(), "{outcome:?}");
        assert!(started.elapsed() < StdDuration::from_secs(5));
    }

    #[tokio::test]
    async fn the_production_cap_keeps_the_last_byte_and_refuses_the_next() {
        let cap = usize::try_from(MAX_DOCUMENT_BYTES).unwrap();
        let (url, _) = serve(http_ok(&vec![b'a'; cap]), false).await;
        let body = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert_eq!(body.unwrap().len(), cap);

        let (url, _) = serve(http_ok(&vec![b'a'; cap + 1]), false).await;
        let outcome = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert!(
            matches!(
                outcome,
                Err(CimdError::Fetch(GuardError::BodyTooLarge { .. }))
            ),
            "{outcome:?}"
        );
    }

    #[tokio::test]
    async fn the_production_timeout_cuts_off_a_server_that_stalls() {
        let (url, _) = serve(Vec::new(), true).await;
        let started = Instant::now();
        let outcome = loopback_fetcher(GuardedFetcher::limits()).fetch(&url).await;
        assert!(outcome.is_err(), "{outcome:?}");
        assert!(
            started.elapsed() < FETCH_TIMEOUT + StdDuration::from_secs(2),
            "the stalled fetch ran {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_server_fault_and_a_caller_mistake_are_told_apart() {
        assert_eq!(
            CimdError::ClientIdMismatch.into_oauth2_error().error,
            "invalid_client"
        );
        assert_eq!(
            CimdError::Fetch(GuardError::DeniedAddress)
                .into_oauth2_error()
                .error,
            "invalid_client"
        );
        assert_eq!(
            CimdError::Store("x".to_owned()).into_oauth2_error().error,
            "server_error"
        );
        assert_eq!(
            CimdError::Status(503).into_oauth2_error().error,
            "temporarily_unavailable"
        );
    }
}
