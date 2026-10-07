// ABOUTME: Overpass API client for route discovery: the public instances, per-attempt timeouts,
// ABOUTME: sequential failover with cooldowns, and a bounded, caller-readable failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::project::user_agent;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::http_client::api_client as shared_client;
use pierre_core::http_client::SharedHttpClient;
use reqwest::header::USER_AGENT;
use reqwest::{StatusCode, Url};
use std::collections::HashMap;
use std::fmt;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, info, warn};

/// Public Overpass instances, tried in order until one answers.
///
/// One entry per independent backend. Each is a keyless, worldwide instance
/// from the OSM wiki's public-instance table whose published policy admits
/// this use (carnet#809):
///
/// - `overpass-api.de` — FOSSGIS, the main instance. Asks for well under
///   10,000 queries a day, no parallel requests, and a pause after a 429.
///   Under load it answers 504 "dispatcher timeout, server too busy" in
///   about 7s — a fast failure rather than a hang.
/// - `overpass.private.coffee` — no rate limit; asks for notice before
///   large-scale use. `overpass.kumi.systems` is a DNS alias (CNAME) of this
///   host, so listing both bought a second wait on the same server.
/// - `maps.mail.ru` — VK Maps: "no requests limitations".
///
/// Instances that need a key (Geofabrik, Overspan, Mapsource, Tracestrack,
/// `NextGIS`, `FairwayMapper`) or that serve only whitelisted callers
/// (`overpass.openstreetmap.fr`) are deliberately absent.
const OVERPASS_ENDPOINTS: &[&str] = &[
    "https://overpass-api.de/api/interpreter",
    "https://overpass.private.coffee/api/interpreter",
    "https://maps.mail.ru/osm/tools/overpass/api/interpreter",
];

/// Seconds the Overpass server is told it may spend on a query, written into
/// every query's `[timeout:N]` header.
///
/// Overpass admits a query only when a slot can hold its declared run time,
/// so a smaller figure is admitted sooner on a loaded server, and it frees
/// the slot once the client has given up anyway. Three-clause queries answer
/// in 2-10s on a healthy instance (see the query-construction notes).
pub(super) const OVERPASS_SERVER_TIMEOUT_SECS: u64 = 20;

/// How long one request to one instance may take, end to end.
///
/// The server's own budget plus room to transfer the body (~560 kB for 300
/// elements with geometry). An instance that holds the connection open
/// without answering is abandoned here instead of at the shared client's
/// 30s timeout.
const OVERPASS_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(OVERPASS_SERVER_TIMEOUT_SECS + 2);

/// Wall-clock budget for the whole walk over every instance.
///
/// A hard ceiling: each attempt's timeout is clipped to what remains, so a
/// tool call fails within this budget whatever the instances do, and the
/// agent can tell the athlete instead of leaving the turn hanging.
const OVERPASS_TOTAL_BUDGET: Duration = Duration::from_secs(45);

/// Pause before the one retry an instance gets after a gateway error.
const OVERPASS_RETRY_PAUSE: Duration = Duration::from_secs(1);

/// How long an instance that timed out, refused the connection or
/// rate-limited us is moved to the back of the order.
///
/// Every `discover_routes` call would otherwise wait out the same dead
/// instance before reaching a live one; a 429 additionally asks for a pause
/// under the main instance's policy.
const OVERPASS_COOLDOWN: Duration = Duration::from_secs(300);

/// Attempts one instance gets in a walk: the first, and one retry after a
/// gateway error.
const MAX_ATTEMPTS_PER_ENDPOINT: u8 = 2;

/// Process-wide record of instances in cooldown, keyed by endpoint URL, with
/// the moment each becomes first-choice again. Process-wide because each
/// `discover_routes` tool call builds a fresh client.
static ENDPOINT_COOLDOWNS: LazyLock<RwLock<HashMap<String, Instant>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// The time limits a walk over the Overpass instances runs under.
#[derive(Debug, Clone, Copy)]
struct OverpassTiming {
    attempt_timeout: Duration,
    total_budget: Duration,
    retry_pause: Duration,
    cooldown: Duration,
}

impl OverpassTiming {
    const PRODUCTION: Self = Self {
        attempt_timeout: OVERPASS_ATTEMPT_TIMEOUT,
        total_budget: OVERPASS_TOTAL_BUDGET,
        retry_pause: OVERPASS_RETRY_PAUSE,
        cooldown: OVERPASS_COOLDOWN,
    };
}

/// Why one request to one Overpass instance produced no usable answer.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AttemptFailure {
    /// 502/503/504: the instance is up and its dispatcher is saturated.
    /// Overpass answers this within seconds, so it earns one retry.
    Busy(StatusCode),
    /// 429: the instance asks us to back off.
    RateLimited(StatusCode),
    /// Any other unsuccessful status.
    Rejected(StatusCode),
    /// No complete answer within the attempt's timeout.
    TimedOut(Duration),
    /// The connection failed or broke off.
    Network(String),
    /// A 200 whose body is not an Overpass JSON response — free instances
    /// answer an overload with an HTML error page.
    Unparseable(String),
}

impl AttemptFailure {
    /// Short label for the structured log field.
    const fn kind(&self) -> &'static str {
        match self {
            Self::Busy(_) => "busy",
            Self::RateLimited(_) => "rate_limited",
            Self::Rejected(_) => "rejected",
            Self::TimedOut(_) => "timeout",
            Self::Network(_) => "network",
            Self::Unparseable(_) => "unparseable",
        }
    }

    /// Whether the instance gets a second attempt in the same walk.
    const fn earns_retry(&self) -> bool {
        matches!(self, Self::Busy(_))
    }

    /// Whether the instance moves to the back of the order for a while: it
    /// cost a full timeout, is unreachable, or asked us to back off.
    const fn cools_down(&self) -> bool {
        matches!(
            self,
            Self::RateLimited(_) | Self::TimedOut(_) | Self::Network(_)
        )
    }
}

impl fmt::Display for AttemptFailure {
    /// The reason as the agent reads it: no HTML, no URL, no stack.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Busy(status) => write!(f, "busy (HTTP {})", status.as_u16()),
            Self::RateLimited(status) => write!(f, "rate limited (HTTP {})", status.as_u16()),
            Self::Rejected(status) => write!(f, "refused (HTTP {})", status.as_u16()),
            Self::TimedOut(after) => write!(f, "no answer within {}s", after.as_secs()),
            Self::Network(_) => f.write_str("connection failed"),
            Self::Unparseable(_) => f.write_str("unreadable response"),
        }
    }
}

/// The host an endpoint URL names, for logs and the caller-facing error.
fn endpoint_host(endpoint: &str) -> String {
    Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| endpoint.to_owned())
}

/// Endpoints in the order to try them: those out of cooldown first, then
/// those still cooling, each group in configured order. A cooling instance
/// is demoted, never dropped — when every instance is cooling, all are
/// still tried.
fn attempt_order<'a>(
    endpoints: &'a [String],
    cooldowns: &HashMap<String, Instant>,
    now: Instant,
) -> Vec<&'a str> {
    let cooling = |endpoint: &str| cooldowns.get(endpoint).is_some_and(|until| *until > now);
    let (ready, demoted): (Vec<&str>, Vec<&str>) = endpoints
        .iter()
        .map(String::as_str)
        .partition(|endpoint| !cooling(endpoint));
    ready.into_iter().chain(demoted).collect()
}

/// Demote an endpoint for `cooldown`.
fn start_cooldown(endpoint: &str, cooldown: Duration) {
    if let Ok(mut cooldowns) = ENDPOINT_COOLDOWNS.write() {
        cooldowns.insert(endpoint.to_owned(), Instant::now() + cooldown);
    }
}

/// Restore an endpoint that answered to its configured place.
fn end_cooldown(endpoint: &str) {
    if let Ok(mut cooldowns) = ENDPOINT_COOLDOWNS.write() {
        cooldowns.remove(endpoint);
    }
}

/// The error a caller sees when no instance answered: every attempt's
/// reason, the instances never reached, and what the agent should do.
fn all_endpoints_failed(
    failures: &[(String, AttemptFailure)],
    untried: &[String],
    budget: Duration,
) -> AppError {
    let mut reasons: Vec<String> = failures
        .iter()
        .map(|(host, failure)| format!("{host}: {failure}"))
        .collect();
    reasons.extend(
        untried
            .iter()
            .map(|host| format!("{host}: not tried, time budget spent")),
    );
    AppError::new(
        ErrorCode::ExternalServiceUnavailable,
        format!(
            "Route search is temporarily unavailable: no public OpenStreetMap (Overpass) \
             server returned routes within {}s ({}). Retry in a few minutes; do not suggest \
             routes from memory.",
            budget.as_secs(),
            reasons.join("; ")
        ),
    )
}

/// Milliseconds since `since`, for a structured log field.
fn elapsed_ms(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// What one walk over the instances has spent and seen so far.
struct EndpointWalk {
    started: Instant,
    budget: Duration,
    failures: Vec<(String, AttemptFailure)>,
    untried: Vec<String>,
}

impl EndpointWalk {
    fn new(budget: Duration) -> Self {
        Self {
            started: Instant::now(),
            budget,
            failures: Vec::new(),
            untried: Vec::new(),
        }
    }

    /// What is left of the budget.
    fn remaining(&self) -> Duration {
        self.budget.saturating_sub(self.started.elapsed())
    }
}

/// Sends one Overpass query to the public instances until one answers.
pub(super) struct OverpassClient {
    client: &'static SharedHttpClient,
    endpoints: Vec<String>,
    timing: OverpassTiming,
}

impl OverpassClient {
    /// A client over the public instances with production time limits.
    pub(super) fn public() -> Self {
        Self {
            client: shared_client(),
            endpoints: OVERPASS_ENDPOINTS.iter().map(|s| (*s).to_owned()).collect(),
            timing: OverpassTiming::PRODUCTION,
        }
    }

    /// Walk the instances until one answers with a body `parse` accepts.
    ///
    /// Sequential, never parallel: the main instance's policy forbids
    /// parallel requests, and racing every volunteer server on each tool
    /// call would multiply the load that makes them fail. Each instance gets
    /// one attempt, plus one retry after a gateway error. Every attempt's
    /// timeout is clipped to the remaining budget, so the walk ends within
    /// [`OverpassTiming::total_budget`] whatever the instances do. Each
    /// failure is logged as a structured warning as it happens. A body
    /// `parse` rejects — an HTML error page served with a 200 — counts as a
    /// failure and the walk moves on.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::ExternalServiceUnavailable`] when no instance answers
    /// within the budget, naming each instance and why it failed.
    pub(super) async fn fetch<T: Send>(
        &self,
        query: &str,
        parse: impl Fn(&str) -> AppResult<T> + Sync,
    ) -> AppResult<T> {
        let mut walk = EndpointWalk::new(self.timing.total_budget);
        let order: Vec<String> = {
            let cooldowns = ENDPOINT_COOLDOWNS
                .read()
                .map(|guard| guard.clone())
                .unwrap_or_default();
            attempt_order(&self.endpoints, &cooldowns, walk.started)
                .into_iter()
                .map(str::to_owned)
                .collect()
        };

        for endpoint in &order {
            if let Some(parsed) = self.walk_endpoint(&mut walk, endpoint, query, &parse).await {
                return Ok(parsed);
            }
        }

        warn!(
            elapsed_ms = elapsed_ms(walk.started),
            attempts = walk.failures.len(),
            untried = walk.untried.len(),
            "Every Overpass instance failed; route discovery unavailable"
        );
        Err(all_endpoints_failed(
            &walk.failures,
            &walk.untried,
            self.timing.total_budget,
        ))
    }

    /// Up to two attempts at one instance — the second only after a gateway
    /// error — each clipped to what remains of the walk's budget. Every
    /// failure is logged as it happens and recorded on `walk`.
    async fn walk_endpoint<T: Send>(
        &self,
        walk: &mut EndpointWalk,
        endpoint: &str,
        query: &str,
        parse: &(impl Fn(&str) -> AppResult<T> + Sync),
    ) -> Option<T> {
        let host = endpoint_host(endpoint);
        for attempt in 1..=MAX_ATTEMPTS_PER_ENDPOINT {
            let remaining = walk.remaining();
            if remaining.is_zero() {
                if attempt == 1 {
                    walk.untried.push(host);
                }
                return None;
            }
            let timeout = remaining.min(self.timing.attempt_timeout);
            let attempt_started = Instant::now();
            let outcome = self
                .attempt(endpoint, query, timeout)
                .await
                .and_then(|body| {
                    parse(&body).map_err(|e| AttemptFailure::Unparseable(e.to_string()))
                });
            let failure = match outcome {
                Ok(parsed) => {
                    end_cooldown(endpoint);
                    info!(
                        endpoint = %host,
                        attempt,
                        elapsed_ms = elapsed_ms(attempt_started),
                        "Overpass instance answered"
                    );
                    return Some(parsed);
                }
                Err(failure) => failure,
            };
            warn!(
                endpoint = %host,
                attempt,
                elapsed_ms = elapsed_ms(attempt_started),
                failure = failure.kind(),
                detail = ?failure,
                "Overpass instance failed"
            );
            if failure.cools_down() {
                start_cooldown(endpoint, self.timing.cooldown);
            }
            let retry = failure.earns_retry() && walk.remaining() > self.timing.retry_pause;
            walk.failures.push((host.clone(), failure));
            if !retry {
                return None;
            }
            sleep(self.timing.retry_pause).await;
        }
        None
    }

    /// One request to one instance, returning its raw body.
    async fn attempt(
        &self,
        endpoint: &str,
        query: &str,
        timeout: Duration,
    ) -> Result<String, AttemptFailure> {
        debug!(endpoint, ?timeout, "Querying Overpass instance");

        let response = self
            .client
            .post(endpoint)
            .header(USER_AGENT, user_agent())
            .timeout(timeout)
            .form(&[("data", query)])
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    AttemptFailure::TimedOut(timeout)
                } else {
                    AttemptFailure::Network(e.to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(match status {
                StatusCode::TOO_MANY_REQUESTS => AttemptFailure::RateLimited(status),
                StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT => AttemptFailure::Busy(status),
                _ => AttemptFailure::Rejected(status),
            });
        }

        response.text().await.map_err(|e| {
            if e.is_timeout() {
                AttemptFailure::TimedOut(timeout)
            } else {
                AttemptFailure::Network(e.to_string())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    /// A well-formed Overpass answer.
    const VALID_BODY: &str = r#"{"elements":[{"type":"way","id":1}]}"#;

    /// What a stub instance does with one request.
    #[derive(Clone, Copy)]
    enum Reply {
        /// Answer with this status and body.
        Respond(u16, &'static str),
        /// Hold the connection open without answering.
        Hang,
    }

    /// A local HTTP server playing one Overpass instance. `script` is the
    /// reply to each successive request; its last entry repeats.
    struct StubInstance {
        endpoint: String,
        hits: Arc<AtomicUsize>,
    }

    impl StubInstance {
        async fn start(script: Vec<Reply>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}/api/interpreter", listener.local_addr().unwrap());
            let hits = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&hits);
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let index = counter.fetch_add(1, Ordering::SeqCst);
                    let reply = script[index.min(script.len() - 1)];
                    tokio::spawn(serve(socket, reply));
                }
            });
            Self { endpoint, hits }
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    async fn serve(mut socket: TcpStream, reply: Reply) {
        read_request(&mut socket).await;
        match reply {
            Reply::Respond(status, body) => {
                let response = format!(
                    "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
            Reply::Hang => sleep(Duration::from_secs(30)).await,
        }
    }

    /// Read one request's headers and its `content-length` body.
    async fn read_request(socket: &mut TcpStream) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let Ok(read) = socket.read(&mut chunk).await else {
                return;
            };
            if read == 0 {
                return;
            }
            buffer.extend_from_slice(&chunk[..read]);
            let text = String::from_utf8_lossy(&buffer);
            let Some(header_end) = text.find("\r\n\r\n") else {
                continue;
            };
            let length = text[..header_end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            if buffer.len() >= header_end + 4 + length {
                return;
            }
        }
    }

    fn service(endpoints: &[&StubInstance], timing: OverpassTiming) -> OverpassClient {
        OverpassClient {
            client: shared_client(),
            endpoints: endpoints.iter().map(|stub| stub.endpoint.clone()).collect(),
            timing,
        }
    }

    /// Timing scaled down so a hung instance costs milliseconds.
    const FAST: OverpassTiming = OverpassTiming {
        attempt_timeout: Duration::from_millis(400),
        total_budget: Duration::from_secs(5),
        retry_pause: Duration::from_millis(10),
        cooldown: Duration::from_secs(300),
    };

    /// Elements in the first answer the walk accepts. The parser stands in
    /// for route ranking: an HTML page fails it as it fails the real one.
    async fn fetch(client: &OverpassClient) -> AppResult<usize> {
        client
            .fetch("[out:json];node(1);out;", |body| {
                let parsed: serde_json::Value = serde_json::from_str(body)
                    .map_err(|e| AppError::internal(format!("not Overpass JSON: {e}")))?;
                parsed["elements"]
                    .as_array()
                    .map(Vec::len)
                    .ok_or_else(|| AppError::internal("no elements array"))
            })
            .await
    }

    #[test]
    fn default_endpoints_are_distinct_backends() {
        let hosts: Vec<String> = OVERPASS_ENDPOINTS
            .iter()
            .map(|endpoint| endpoint_host(endpoint))
            .collect();
        let mut distinct = hosts.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), hosts.len(), "duplicate host in {hosts:?}");
        // kumi.systems is a CNAME of private.coffee: one server, not two.
        assert!(
            !hosts.iter().any(|host| host == "overpass.kumi.systems"),
            "overpass.kumi.systems aliases overpass.private.coffee: {hosts:?}"
        );
    }

    #[test]
    fn production_attempts_fit_the_budget_and_the_shared_client() {
        // The server is told to stop before the client gives up, and one
        // attempt never outlasts the shared client's 30s ceiling.
        assert!(Duration::from_secs(OVERPASS_SERVER_TIMEOUT_SECS) < OVERPASS_ATTEMPT_TIMEOUT);
        assert!(OVERPASS_ATTEMPT_TIMEOUT < Duration::from_secs(30));
        assert!(OVERPASS_ATTEMPT_TIMEOUT < OVERPASS_TOTAL_BUDGET);
    }

    #[test]
    fn cooling_endpoints_are_demoted_not_dropped() {
        let endpoints: Vec<String> = ["https://a/i", "https://b/i", "https://c/i"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let now = Instant::now();
        let ahead = now + Duration::from_secs(60);
        let mut cooldowns = HashMap::new();
        cooldowns.insert("https://a/i".to_owned(), ahead);
        // A cooldown that has run out no longer demotes.
        cooldowns.insert("https://b/i".to_owned(), now);
        assert_eq!(
            attempt_order(&endpoints, &cooldowns, now),
            vec!["https://b/i", "https://c/i", "https://a/i"]
        );
        cooldowns.insert("https://b/i".to_owned(), ahead);
        cooldowns.insert("https://c/i".to_owned(), ahead);
        assert_eq!(
            attempt_order(&endpoints, &cooldowns, now),
            vec!["https://a/i", "https://b/i", "https://c/i"],
            "every instance cooling: all still tried, in configured order"
        );
    }

    #[tokio::test]
    async fn a_busy_instance_is_retried_once_then_the_next_answers() {
        let busy = StubInstance::start(vec![Reply::Respond(504, "too busy")]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let elements = fetch(&service(&[&busy, &healthy], FAST)).await.unwrap();
        assert_eq!(elements, 1);
        assert_eq!(busy.hits(), 2, "a gateway error earns exactly one retry");
        assert_eq!(healthy.hits(), 1);
    }

    #[tokio::test]
    async fn a_busy_instance_that_recovers_on_retry_answers() {
        let flaky = StubInstance::start(vec![
            Reply::Respond(504, "too busy"),
            Reply::Respond(200, VALID_BODY),
        ])
        .await;
        let next = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let elements = fetch(&service(&[&flaky, &next], FAST)).await.unwrap();
        assert_eq!(elements, 1);
        assert_eq!(flaky.hits(), 2);
        assert_eq!(next.hits(), 0, "the walk stops at the first answer");
    }

    #[tokio::test]
    async fn rate_limits_and_html_pages_fall_through_without_retry() {
        let limited = StubInstance::start(vec![Reply::Respond(429, "slow down")]).await;
        let html = StubInstance::start(vec![Reply::Respond(200, "<html>busy</html>")]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let elements = fetch(&service(&[&limited, &html, &healthy], FAST))
            .await
            .unwrap();
        assert_eq!(elements, 1);
        assert_eq!(limited.hits(), 1, "a 429 is never retried");
        assert_eq!(html.hits(), 1);
        assert_eq!(healthy.hits(), 1);
    }

    #[tokio::test]
    async fn a_hung_instance_is_abandoned_and_demoted_for_the_next_call() {
        let hung = StubInstance::start(vec![Reply::Hang]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let service = service(&[&hung, &healthy], FAST);

        let started = Instant::now();
        fetch(&service).await.unwrap();
        assert!(
            started.elapsed() < FAST.attempt_timeout * 3,
            "the hung instance held the walk for {:?}",
            started.elapsed()
        );
        assert_eq!(hung.hits(), 1);

        // Cooling: the next call reaches the healthy instance first.
        fetch(&service).await.unwrap();
        assert_eq!(hung.hits(), 1, "a cooling instance is not tried first");
        assert_eq!(healthy.hits(), 2);
    }

    #[tokio::test]
    async fn every_instance_failing_is_a_clear_unavailable_error_within_budget() {
        let busy = StubInstance::start(vec![Reply::Respond(504, "<html>busy</html>")]).await;
        let hung = StubInstance::start(vec![Reply::Hang]).await;
        let unreached = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let timing = OverpassTiming {
            attempt_timeout: Duration::from_millis(300),
            total_budget: Duration::from_millis(300),
            retry_pause: Duration::from_millis(10),
            cooldown: Duration::from_secs(300),
        };

        let started = Instant::now();
        let err = fetch(&service(&[&busy, &hung, &unreached], timing))
            .await
            .unwrap_err();
        let elapsed = started.elapsed();

        assert_eq!(err.code, ErrorCode::ExternalServiceUnavailable);
        assert!(
            elapsed < timing.total_budget + Duration::from_millis(250),
            "the walk overran its budget: {elapsed:?}"
        );
        assert_eq!(busy.hits(), 2);
        assert_eq!(
            unreached.hits(),
            0,
            "no attempt starts once the budget is spent"
        );
        let message = err.to_string();
        assert!(message.contains("busy (HTTP 504)"), "{message}");
        assert!(message.contains("no answer within"), "{message}");
        assert!(
            message.contains("not tried, time budget spent"),
            "{message}"
        );
        assert!(
            message.contains("do not suggest routes from memory"),
            "{message}"
        );
        assert!(!message.contains("<html>"), "raw HTML leaked: {message}");
    }
}
