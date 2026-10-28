// ABOUTME: Fetching an athlete's export only through tronc's archive_fetch guard: https, listed hosts, pinned IPs, caps
// ABOUTME: A check before the job (reachable, how large) and the streamed download a step spools; never a bare client

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Fetching an export
//!
//! The link is the athlete's, so the fetch is held to tronc's
//! [`FetchPolicy::archive_fetch`]: `https` on the default port only, no
//! credentials in the URL, a host on an exact allowlist, every redirect hop
//! held to the same rules, the connection made to the address the name
//! resolved to and checked against the deny list, and byte, idle and total
//! time caps. The allowlist is the deployment's
//! ([`ENV_ARCHIVE_FETCH_HOSTS`]); with none set, every link is refused, so a
//! deployment imports nothing until its operator names the hosts a service's
//! exports are served from.
//!
//! Nothing here ever logs or returns the link: tronc's errors carry none of
//! it, and callers log the link's digest.

use std::env;
use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use dravr_barrage::import::archive::ArchiveLimits;
use dravr_barrage::{BarrageError, FailureCode};
use dravr_tronc::http_client::guard::{
    FetchLimits, FetchPolicy, GuardError, GuardedClient, GuardedResponse, HostAllowlist,
};
use futures_util::future;
use futures_util::stream::{self, BoxStream, StreamExt};
use reqwest::header::{HeaderValue, CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use reqwest::{Method, StatusCode};

/// The hosts an export link may be fetched from, comma-separated and matched
/// exactly. Unset or empty, no link is fetched.
pub const ENV_ARCHIVE_FETCH_HOSTS: &str = "ARCHIVE_FETCH_HOSTS";

/// The largest export downloaded: the archive barrage spools, never less. A
/// real Strava export of ten years of recordings and photos is 6.3 GB
/// (carnet#1215), past the guard's default of 1 GiB for an archive fetch.
fn max_download_bytes() -> u64 {
    ArchiveLimits::default().max_archive_bytes
}

/// The longest a whole download may take: inside the step delivery that runs
/// it (`runner::STEP_DISPATCH_DEADLINE`, twenty minutes), so a stalled export
/// fails its step rather than outliving it.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(18);

/// Why a pasted link was refused before any job was created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkRefusal {
    /// Not an `https` URL on the default port without credentials.
    Invalid,
    /// Its host is not one exports are fetched from.
    HostNotAllowed,
    /// The service no longer serves the export behind it.
    Expired,
    /// It could not be reached.
    Unreachable,
}

impl LinkRefusal {
    /// The stable code the API answers with.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "link_invalid",
            Self::HostNotAllowed => "link_host_not_allowed",
            Self::Expired => "link_expired",
            Self::Unreachable => "link_unreachable",
        }
    }

    fn of_guard(error: &GuardError) -> Self {
        match error {
            GuardError::HostNotAllowed => Self::HostNotAllowed,
            GuardError::InvalidUrl
            | GuardError::SchemeNotAllowed
            | GuardError::CredentialsInUrl
            | GuardError::PortNotAllowed
            | GuardError::MissingHost
            | GuardError::LocalName
            | GuardError::DeniedAddress => Self::Invalid,
            _ => Self::Unreachable,
        }
    }

    fn of_status(status: StatusCode) -> Self {
        if is_expired(status) {
            Self::Expired
        } else {
            Self::Unreachable
        }
    }
}

impl fmt::Display for LinkRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// What the check of a link found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkCheck {
    /// The export's size, when the service says.
    pub size_bytes: Option<u64>,
}

/// How an export is fetched: through the guard in production, from a fixture
/// in a test.
#[async_trait]
pub trait ExportFetcher: Send + Sync {
    /// Check that `link` may be fetched and serves an export, reading none of
    /// it.
    async fn check(&self, link: &str) -> Result<LinkCheck, LinkRefusal>;

    /// The export `link` serves, as the stream barrage spools.
    ///
    /// A service that no longer serves it ends the stream with a
    /// [`FailureCode::LinkExpired`] archive error; any other failure with
    /// [`FailureCode::DownloadFailed`].
    async fn download(&self, link: &str) -> BoxStream<'static, Result<Bytes, BarrageError>>;
}

/// The production fetcher: tronc's guarded client under `archive_fetch`.
pub struct GuardedExportFetcher {
    client: GuardedClient,
}

impl fmt::Debug for GuardedExportFetcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuardedExportFetcher")
            .field("limits", &self.client.limits())
            .finish()
    }
}

impl GuardedExportFetcher {
    /// A fetcher admitting the hosts in [`ENV_ARCHIVE_FETCH_HOSTS`].
    ///
    /// # Errors
    ///
    /// The guard's error when the client cannot be built.
    pub fn try_from_env() -> Result<Self, GuardError> {
        let hosts = env::var(ENV_ARCHIVE_FETCH_HOSTS).unwrap_or_default();
        Self::try_new(HostAllowlist::from_comma_separated(&hosts))
    }

    /// A fetcher admitting `hosts`.
    ///
    /// # Errors
    ///
    /// The guard's error when the client cannot be built.
    pub fn try_new(hosts: HostAllowlist) -> Result<Self, GuardError> {
        let policy = FetchPolicy::archive_fetch(hosts);
        let limits = FetchLimits {
            max_body_bytes: max_download_bytes(),
            total_timeout: DOWNLOAD_TIMEOUT,
            ..policy.default_limits()
        };
        Ok(Self {
            client: GuardedClient::builder(policy).limits(limits).build()?,
        })
    }

    async fn send(
        &self,
        method: Method,
        link: &str,
        range: Option<&'static str>,
    ) -> Result<GuardedResponse, GuardError> {
        let mut request = self.client.request(method, link)?;
        if let Some(range) = range {
            request = request.header(RANGE, HeaderValue::from_static(range));
        }
        request.send().await
    }
}

#[async_trait]
impl ExportFetcher for GuardedExportFetcher {
    async fn check(&self, link: &str) -> Result<LinkCheck, LinkRefusal> {
        self.client
            .check_url(link)
            .map_err(|e| LinkRefusal::of_guard(&e))?;
        // A one-byte ranged GET, not a HEAD: a presigned URL is signed for
        // the method it was made for, and an export link is made for GET.
        let response = self
            .send(Method::GET, link, Some("bytes=0-0"))
            .await
            .map_err(|e| LinkRefusal::of_guard(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(LinkRefusal::of_status(status));
        }
        Ok(LinkCheck {
            size_bytes: total_size(&response),
        })
    }

    async fn download(&self, link: &str) -> BoxStream<'static, Result<Bytes, BarrageError>> {
        let response = match self.send(Method::GET, link, None).await {
            Ok(response) => response,
            Err(e) => return failed(FailureCode::DownloadFailed, &e.to_string()),
        };
        let status = response.status();
        if is_expired(status) {
            return failed(
                FailureCode::LinkExpired,
                &format!("the export answered {status}"),
            );
        }
        if !status.is_success() {
            return failed(
                FailureCode::DownloadFailed,
                &format!("the export answered {status}"),
            );
        }
        stream::unfold(Some(response), |response| async move {
            let mut response = response?;
            match response.chunk().await {
                Ok(Some(chunk)) => Some((Ok(chunk), Some(response))),
                Ok(None) => None,
                Err(e) => Some((
                    Err(BarrageError::archive(
                        FailureCode::DownloadFailed,
                        e.to_string(),
                    )),
                    None,
                )),
            }
        })
        .boxed()
    }
}

/// Whether `status` says the export is no longer served behind the link.
fn is_expired(status: StatusCode) -> bool {
    matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND | StatusCode::GONE
    )
}

/// A stream that fails at once with `code`.
fn failed(code: FailureCode, detail: &str) -> BoxStream<'static, Result<Bytes, BarrageError>> {
    stream::once(future::ready(Err(BarrageError::archive(code, detail)))).boxed()
}

/// The whole export's size a ranged answer states (`Content-Range: bytes
/// 0-0/<total>`), or a whole answer's length.
fn total_size(response: &GuardedResponse) -> Option<u64> {
    let headers = response.headers();
    if let Some(range) = headers.get(CONTENT_RANGE).and_then(|v| v.to_str().ok()) {
        return range
            .rsplit_once('/')
            .and_then(|(_, total)| total.parse().ok());
    }
    headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_link_off_the_allowlist_or_not_https_is_refused_before_any_fetch() {
        let fetcher = GuardedExportFetcher::try_new(HostAllowlist::new(["export.example.com"]))
            .expect("client");
        assert_eq!(
            fetcher
                .check("https://elsewhere.example.com/export.zip")
                .await,
            Err(LinkRefusal::HostNotAllowed),
            "a host off the list is refused"
        );
        assert_eq!(
            fetcher.check("http://export.example.com/export.zip").await,
            Err(LinkRefusal::Invalid),
            "plain http is refused"
        );
        assert_eq!(
            fetcher
                .check("https://user:pw@export.example.com/export.zip")
                .await,
            Err(LinkRefusal::Invalid),
            "credentials in the URL are refused"
        );
    }

    #[test]
    fn an_export_as_large_as_barrage_spools_is_downloaded_whole() {
        let fetcher = GuardedExportFetcher::try_new(HostAllowlist::new(["export.example.com"]))
            .expect("client");
        // Both sides are read, not a literal: the download cap and the spool
        // cap are two artefacts nothing else keeps equal.
        assert_eq!(
            fetcher.client.limits().max_body_bytes,
            ArchiveLimits::default().max_archive_bytes,
            "bug: the download stops short of an export barrage would import"
        );
        assert!(
            fetcher.client.limits().max_body_bytes > 6_303_666_727,
            "bug: a real 6.3 GB Strava export would be cut off"
        );
    }

    #[tokio::test]
    async fn with_no_hosts_configured_every_link_is_refused() {
        let fetcher = GuardedExportFetcher::try_new(HostAllowlist::new(Vec::<String>::new()))
            .expect("client");
        assert_eq!(
            fetcher.check("https://export.example.com/export.zip").await,
            Err(LinkRefusal::HostNotAllowed),
            "an empty allowlist admits nothing"
        );
    }

    #[test]
    fn a_service_that_stopped_serving_the_export_reads_as_an_expired_link() {
        for status in [
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::GONE,
        ] {
            assert_eq!(
                LinkRefusal::of_status(status),
                LinkRefusal::Expired,
                "{status}"
            );
        }
        assert_eq!(
            LinkRefusal::of_status(StatusCode::BAD_GATEWAY),
            LinkRefusal::Unreachable,
            "a server error is not an expiry"
        );
    }
}
