// ABOUTME: Overpass API client for route discovery: the public instances, per-attempt timeouts,
// ABOUTME: hedged failover with cooldowns, and a bounded, caller-readable failure
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use futures_util::stream::{FuturesUnordered, StreamExt};
use pierre_core::errors::AppResult;
use pierre_core::http_client::api_client as shared_client;
use pierre_core::http_client::SharedHttpClient;
use reqwest::Url;
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};
use tokio::select;
use tokio::time::sleep;
use tracing::{info, warn};

mod attempt;
mod hedge;
mod unavailable;

use attempt::{walk_outcomes, AttemptFailure};
use hedge::{OVERPASS_CALM_HEDGE_DELAY, OVERPASS_HEDGE_DELAY};

use unavailable::{
    held_unavailable, hold_unavailable, WalkCause, OVERPASS_UNAVAILABLE_HOLD,
    ROUTE_SEARCH_HELD_EVENT, ROUTE_SEARCH_UNAVAILABLE_EVENT,
};

/// Public Overpass instances, asked in order until one answers.
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

/// The path an Overpass instance answers queries at, under an origin a
/// deployment names.
const INTERPRETER_PATH: &str = "/api/interpreter";

/// Seconds the Overpass server is told it may spend on a query, written into
/// every query's `[timeout:N]` header.
///
/// Overpass admits a query only when a slot can hold its declared run time,
/// so a smaller figure is admitted sooner on a loaded server, and it frees
/// the slot once the client has given up anyway. Three-clause queries answer
/// in 2-10s on a healthy instance (see the query-construction notes); the
/// last instance a walk asks still has this long and its queue wait before
/// the walk's budget ends.
pub(super) const OVERPASS_SERVER_TIMEOUT_SECS: u64 = 4;

/// Wall-clock budget for the whole walk over every instance.
///
/// A hard ceiling: each attempt's timeout is what remains of it, so a tool
/// call fails within this budget whatever the instances do, and the agent can
/// tell the athlete instead of leaving the turn hanging. On 2026-11-19 every
/// public instance hung, and 38 s attempts (carnet#1121) cost the turn 78 s
/// and its forecast.
///
/// A few seconds (carnet#1665): the route search must leave the turn the
/// time to read the forecast and sunset of the session it plans. Every
/// instance asked runs until this budget ends, so the last one hedged in
/// still has most of it; an instance that timed out, or was asked before the
/// one that answered and was still silent, is cooled down, so the next call
/// asks another one first.
///
/// The per-request timeout set in `attempt` replaces, for this request, the
/// shared client's timeout configured at startup (`API_CLIENT_TIMEOUT_SECS`):
/// reqwest reads a request's own timeout before the client's.
const OVERPASS_TOTAL_BUDGET: Duration = Duration::from_secs(8);

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
    hedge_delay: Duration,
    calm_hedge_delay: Duration,
    total_budget: Duration,
    retry_pause: Duration,
    cooldown: Duration,
    unavailable_hold: Duration,
}

impl OverpassTiming {
    const PRODUCTION: Self = Self {
        hedge_delay: OVERPASS_HEDGE_DELAY,
        calm_hedge_delay: OVERPASS_CALM_HEDGE_DELAY,
        total_budget: OVERPASS_TOTAL_BUDGET,
        retry_pause: OVERPASS_RETRY_PAUSE,
        cooldown: OVERPASS_COOLDOWN,
        unavailable_hold: OVERPASS_UNAVAILABLE_HOLD,
    };
}

/// The host an endpoint URL names, for the logs.
fn endpoint_host(endpoint: &str) -> String {
    Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| endpoint.to_owned())
}

/// Endpoints in the order to ask them: those out of cooldown first, then
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
}

/// What one instance's turn in a walk came to.
struct Lane<T> {
    /// The instance's place in the walk's order.
    index: usize,
    /// The parsed answer, when it gave one.
    answer: Option<T>,
    /// Each failed attempt, with its host.
    failures: Vec<(String, AttemptFailure)>,
    /// Its host, when the budget was spent before it was asked.
    untried: Option<String>,
}

/// The instances a client at `origin` walks: see [`OverpassClient::at`].
fn endpoints_at(origin: Option<&str>) -> Vec<String> {
    origin.map_or_else(
        || OVERPASS_ENDPOINTS.iter().map(|s| (*s).to_owned()).collect(),
        |origin| {
            vec![format!(
                "{}{INTERPRETER_PATH}",
                origin.trim_end_matches('/')
            )]
        },
    )
}

/// Sends one Overpass query to the public instances until one answers.
#[derive(Debug)]
pub(super) struct OverpassClient {
    client: &'static SharedHttpClient,
    endpoints: Vec<String>,
    timing: OverpassTiming,
}

impl OverpassClient {
    /// A client with production time limits over the instances `origin`
    /// names.
    ///
    /// No origin is the public instances, `overpass-api.de` first: what a
    /// deployment asks. An origin, a scheme and authority, is the single
    /// instance queried at its `/api/interpreter`; the slashes it ends in are
    /// dropped first. A test configuration names a refused or loopback one,
    /// so no query leaves the machine (carnet#1626).
    pub(super) fn at(origin: Option<&str>) -> Self {
        Self {
            client: shared_client(),
            endpoints: endpoints_at(origin),
            timing: OverpassTiming::PRODUCTION,
        }
    }

    /// Walk the instances until one answers with a body `parse` accepts.
    ///
    /// Hedged: the next instance is asked when the ones in flight have been
    /// silent for the hedge ([`OverpassTiming::hedge_delay`] after a recent
    /// failure, [`OverpassTiming::calm_hedge_delay`] otherwise), or at once
    /// when one fails,
    /// and the first accepted answer ends the walk, dropping the requests
    /// still open. An instance never has two requests in flight (the main
    /// instance's policy forbids parallel requests), and on a healthy day
    /// the first answers before the hedge, so it is the only one asked.
    /// Each instance gets one attempt, plus one retry after a gateway error.
    /// Every attempt's timeout is what remains of the budget, so the walk
    /// ends within [`OverpassTiming::total_budget`] whatever the instances
    /// do. Each failure is logged as a structured warning as it happens. A body
    /// `parse` rejects — an HTML error page served with a 200 — counts as a
    /// failure and the walk moves on, as does a 200 whose `remark` says the
    /// server stopped the query: its elements are partial, so never parsed.
    ///
    /// # Errors
    ///
    /// [`pierre_core::errors::ErrorCode::ExternalServiceUnavailable`] when no instance answers
    /// within the budget, worded for the athlete: overloaded when every
    /// instance said it was saturated, a plain failure otherwise. Each
    /// instance and why it failed are in the warning logged with it. For
    /// [`OverpassTiming::unavailable_hold`] after such a walk that asked every
    /// instance, the same instances are not asked again and the same error
    /// answers at once.
    pub(super) async fn fetch<T: Send>(
        &self,
        query: &str,
        parse: impl Fn(&str) -> AppResult<T> + Sync,
    ) -> AppResult<T> {
        if let Some(cause) = held_unavailable(&self.endpoints, Instant::now()) {
            info!(
                event = ROUTE_SEARCH_HELD_EVENT,
                cause = cause.label(),
                "Route search still unavailable: the last walk found no instance answering moments ago"
            );
            return Err(cause.error());
        }
        let mut walk = EndpointWalk::new(self.timing.total_budget);
        let (order, hedge_delay) = {
            let cooldowns = ENDPOINT_COOLDOWNS
                .read()
                .map_or_default(|guard| guard.clone());
            let order: Vec<String> = attempt_order(&self.endpoints, &cooldowns, walk.started)
                .into_iter()
                .map(str::to_owned)
                .collect();
            (order, self.hedge_delay(&cooldowns, walk.started))
        };

        let deadline = walk.started + walk.budget;
        // Each instance's turn, not yet started: a future runs only once the
        // walk moves it into `lanes`.
        let mut queued = order
            .iter()
            .enumerate()
            .map(|(index, endpoint)| self.walk_endpoint(index, endpoint, query, &parse, deadline));
        let mut finished = vec![false; order.len()];
        let mut lanes = FuturesUnordered::new();
        lanes.extend(queued.next());
        loop {
            select! {
                // An answer ready at the same moment as the hedge wins: the
                // hedge would otherwise ask one more instance for nothing.
                biased;
                done = lanes.next() => {
                    let Some(lane) = done else { break };
                    finished[lane.index] = true;
                    if let Some(parsed) = lane.answer {
                        // An instance asked earlier and still silent is as
                        // slow as one that timed out: the next call asks the
                        // one that answered first.
                        for (index, endpoint) in order.iter().enumerate().take(lane.index) {
                            if !finished[index] {
                                start_cooldown(endpoint, self.timing.cooldown);
                            }
                        }
                        return Ok(parsed);
                    }
                    walk.failures.extend(lane.failures);
                    walk.untried.extend(lane.untried);
                    lanes.extend(queued.next());
                }
                () = sleep(hedge_delay), if queued.len() > 0 => {
                    lanes.extend(queued.next());
                }
            }
        }

        // The one line a log-based metric counts (route_search_monitoring.tf
        // filters on its `event`); the per-host detail stays here.
        let cause = WalkCause::of(&walk.failures);
        warn!(
            event = ROUTE_SEARCH_UNAVAILABLE_EVENT,
            cause = cause.label(),
            elapsed_ms = elapsed_ms(walk.started),
            budget_ms = u64::try_from(walk.budget.as_millis()).unwrap_or(u64::MAX),
            attempts = walk.failures.len(),
            untried = walk.untried.len(),
            outcomes = %walk_outcomes(&walk.failures, &walk.untried),
            "Route search unavailable: no public Overpass instance answered"
        );
        // Held only when every instance was asked: one the budget never
        // reached is out of cooldown, so the next walk asks it first and a
        // retry is not a repeat to the instances that just failed.
        if walk.untried.is_empty() {
            hold_unavailable(&self.endpoints, cause, self.timing.unavailable_hold);
        }
        Err(cause.error())
    }

    /// The hedge this walk runs under: see [`hedge::delay`].
    fn hedge_delay(&self, cooldowns: &HashMap<String, Instant>, now: Instant) -> Duration {
        let troubled = hedge::troubled(&self.endpoints, cooldowns, now, self.timing.cooldown);
        hedge::delay(
            troubled,
            self.timing.hedge_delay,
            self.timing.calm_hedge_delay,
        )
    }

    /// Up to two attempts at one instance — the second only after a gateway
    /// error — each lasting at most until `deadline`. Every failure is logged
    /// as it happens and recorded on the lane.
    async fn walk_endpoint<T: Send>(
        &self,
        index: usize,
        endpoint: &str,
        query: &str,
        parse: &(impl Fn(&str) -> AppResult<T> + Sync),
        deadline: Instant,
    ) -> Lane<T> {
        let host = endpoint_host(endpoint);
        let mut lane = Lane {
            index,
            answer: None,
            failures: Vec::new(),
            untried: None,
        };
        for attempt in 1..=MAX_ATTEMPTS_PER_ENDPOINT {
            let timeout = deadline.saturating_duration_since(Instant::now());
            if timeout.is_zero() {
                if attempt == 1 {
                    lane.untried = Some(host);
                }
                return lane;
            }
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
                    lane.answer = Some(parsed);
                    return lane;
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
            let retry = failure.earns_retry()
                && deadline.saturating_duration_since(Instant::now()) > self.timing.retry_pause;
            lane.failures.push((host.clone(), failure));
            if !retry {
                return lane;
            }
            sleep(self.timing.retry_pause).await;
        }
        lane
    }
}

#[cfg(test)]
mod tests {
    use super::attempt::cut_short_remark;
    use super::unavailable::{ROUTE_SEARCH_FAILED, ROUTE_SEARCH_OVERLOADED};
    use super::*;
    use pierre_core::errors::{AppError, ErrorCode};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::time;

    /// A well-formed Overpass answer.
    const VALID_BODY: &str = r#"{"elements":[{"type":"way","id":1}]}"#;

    /// What a stub instance does with one request.
    #[derive(Clone, Copy)]
    enum Reply {
        /// Answer with this status and body.
        Respond(u16, &'static str),
        /// Hold the connection open without answering.
        Hang,
        /// Answer this status and body after this many milliseconds.
        After(u64, u16, &'static str),
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
            Reply::Respond(status, body) => respond(socket, status, body).await,
            Reply::Hang => sleep(Duration::from_secs(30)).await,
            Reply::After(millis, status, body) => {
                sleep(Duration::from_millis(millis)).await;
                respond(socket, status, body).await;
            }
        }
    }

    async fn respond(mut socket: TcpStream, status: u16, body: &str) {
        let response = format!(
            "HTTP/1.1 {status} Stub\r\ncontent-type: application/json\r\n\
             content-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        // The stub has nobody to report a write failure to; each test
        // asserts on what the client received.
        if socket.write_all(response.as_bytes()).await.is_ok() {
            drop(socket.shutdown().await);
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
        hedge_delay: Duration::from_millis(200),
        calm_hedge_delay: Duration::from_millis(200),
        total_budget: Duration::from_secs(5),
        retry_pause: Duration::from_millis(10),
        cooldown: Duration::from_secs(300),
        unavailable_hold: Duration::from_secs(300),
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
    fn no_origin_is_the_public_instances_and_an_origin_is_the_only_one() {
        assert_eq!(
            endpoints_at(None),
            OVERPASS_ENDPOINTS
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>(),
            "bug: no origin no longer walks every public instance"
        );
        assert_eq!(
            endpoints_at(None).first().map(String::as_str),
            Some("https://overpass-api.de/api/interpreter"),
            "a deployment asks the main instance first"
        );
        for written in ["http://127.0.0.1:9", "http://127.0.0.1:9//"] {
            assert_eq!(
                endpoints_at(Some(written)),
                ["http://127.0.0.1:9/api/interpreter"],
                "bug: the origin {written} is not the single instance asked"
            );
        }
    }

    #[test]
    fn production_timing_leaves_the_last_instance_a_full_query() {
        // Hung instances ahead of it hedge the last one in one delay apart,
        // and it still has the server's whole query time before the walk's
        // budget ends (carnet#1665 bounds the walk to a few seconds).
        let hedges = u32::try_from(OVERPASS_ENDPOINTS.len() - 1).unwrap();
        let last_window = OVERPASS_TOTAL_BUDGET.saturating_sub(OVERPASS_HEDGE_DELAY * hedges);
        assert!(
            Duration::from_secs(OVERPASS_SERVER_TIMEOUT_SECS) < last_window,
            "bug: the last instance asked must have the server's query time: {last_window:?}"
        );
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
            started.elapsed() < FAST.hedge_delay * 3,
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
        // A hedge longer than the budget: the third instance is reached only
        // once the second has spent it.
        let timing = OverpassTiming {
            hedge_delay: Duration::from_secs(1),
            calm_hedge_delay: Duration::from_secs(1),
            total_budget: Duration::from_millis(300),
            retry_pause: Duration::from_millis(10),
            cooldown: Duration::from_secs(300),
            unavailable_hold: Duration::from_secs(300),
        };

        let started = Instant::now();
        let err = fetch(&service(&[&busy, &hung, &unreached], timing))
            .await
            .unwrap_err();
        let elapsed = started.elapsed();

        assert_eq!(err.code, ErrorCode::ExternalServiceUnavailable);
        // The budget itself is pinned by `unreached` never being asked and by
        // the error naming it as not tried. This bound catches the walk that
        // ignores its budget and waits on the hung instance forever;
        // a 250 ms margin also caught a loaded CI runner's scheduling jitter
        // (main, 3202e3d3d), so the margin is wide enough for a busy runner
        // and still far short of a hang.
        assert!(
            elapsed < timing.total_budget + Duration::from_secs(2),
            "the walk overran its budget: {elapsed:?}"
        );
        assert_eq!(busy.hits(), 2);
        assert_eq!(
            unreached.hits(),
            0,
            "no attempt starts once the budget is spent"
        );
        let message = err.to_string();
        assert!(
            message.contains("do not suggest routes from memory"),
            "{message}"
        );
        assert!(!message.contains("<html>"), "raw HTML leaked: {message}");
        assert_athlete_facing(&message);
        assert_eq!(
            err.message, ROUTE_SEARCH_OVERLOADED,
            "bug: busy and hung instances are the overloaded message"
        );
    }

    /// What the athlete, and the model relaying it, may read when the route
    /// search is unavailable: plain words about the public service, never a
    /// host, a status code or a timing (carnet#1705).
    fn assert_athlete_facing(message: &str) {
        for technical in ["127.0.0.1", "HTTP", "within", "not tried", "s)", "Overpass"] {
            assert!(
                !message.contains(technical),
                "bug: the athlete-facing message names {technical:?}: {message}"
            );
        }
        assert!(
            message.contains("OpenStreetMap"),
            "bug: the message says what is down: {message}"
        );
        assert!(
            message.contains("again later"),
            "bug: the message says trying again later may work: {message}"
        );
    }

    #[test]
    fn both_unavailable_messages_are_athlete_facing() {
        assert_athlete_facing(ROUTE_SEARCH_OVERLOADED);
        assert_athlete_facing(ROUTE_SEARCH_FAILED);
    }

    // A 400 is our query refused: the athlete is not told the public service
    // is overloaded.
    #[tokio::test]
    async fn a_refused_query_is_not_told_as_an_overload() {
        let refusing = StubInstance::start(vec![Reply::Respond(400, "bad query")]).await;

        let err = fetch(&service(&[&refusing], FAST))
            .await
            .expect_err("bug: a refused query lists no route");

        assert_eq!(err.code, ErrorCode::ExternalServiceUnavailable);
        assert_eq!(
            err.message, ROUTE_SEARCH_FAILED,
            "bug: a refused query is the plain failure, not an overload"
        );
        assert_eq!(refusing.hits(), 1, "bug: a 400 is never retried");
    }

    // An agent retrying within seconds gets the same answer without the
    // saturated instances being asked again.
    #[tokio::test]
    async fn a_failed_walk_answers_the_next_search_without_asking_again() {
        let busy = StubInstance::start(vec![Reply::Respond(504, "too busy")]).await;
        let client = service(&[&busy], FAST);

        let first = fetch(&client)
            .await
            .expect_err("bug: a busy instance answers");
        assert_eq!(busy.hits(), 2, "a gateway error earns exactly one retry");
        let second = fetch(&client)
            .await
            .expect_err("bug: the held failure answers the retry");

        assert_eq!(
            busy.hits(),
            2,
            "bug: a search within the hold asked the instance again"
        );
        assert_eq!(
            second.message, first.message,
            "bug: the held answer is the walk's own"
        );
        assert_eq!(second.code, ErrorCode::ExternalServiceUnavailable);
    }

    #[tokio::test]
    async fn the_instances_are_asked_again_once_the_hold_ends() {
        let recovering = StubInstance::start(vec![
            Reply::Respond(504, "too busy"),
            Reply::Respond(504, "too busy"),
            Reply::Respond(200, VALID_BODY),
        ])
        .await;
        let timing = OverpassTiming {
            unavailable_hold: Duration::from_millis(50),
            ..FAST
        };
        let client = service(&[&recovering], timing);

        fetch(&client)
            .await
            .expect_err("bug: both attempts were busy");
        sleep(Duration::from_millis(100)).await;
        let elements = fetch(&client)
            .await
            .expect("bug: a recovered instance answers once the hold ends");

        assert_eq!(elements, 1);
        assert_eq!(recovering.hits(), 3, "bug: the hold outlived its length");
    }

    // A calm walk can spend its budget on hung instances before reaching the
    // one answering (2026-11-27); holding it would keep the retry from it.
    #[tokio::test]
    async fn a_walk_that_left_an_instance_untried_is_not_held() {
        let hung = StubInstance::start(vec![Reply::Hang]).await;
        let answering = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        // A hedge longer than the budget: the first walk never asks the second.
        let timing = OverpassTiming {
            hedge_delay: Duration::from_secs(1),
            calm_hedge_delay: Duration::from_secs(1),
            total_budget: Duration::from_millis(300),
            ..FAST
        };
        let client = service(&[&hung, &answering], timing);

        fetch(&client)
            .await
            .expect_err("bug: the hung instance spent the budget");
        let elements = fetch(&client)
            .await
            .expect("bug: the retry was held although an instance was never asked");

        assert_eq!(elements, 1, "bug: the untried instance's answer is used");
        assert_eq!(
            (hung.hits(), answering.hits()),
            (1, 1),
            "bug: the retry asks the untried instance first, the hung one is cooling"
        );
    }

    #[test]
    fn only_a_runtime_error_remark_cuts_an_answer_short() {
        assert!(cut_short_remark(TIMED_OUT_BODY).is_some());
        assert!(cut_short_remark(OUT_OF_MEMORY_BODY).is_some());
        assert_eq!(cut_short_remark(VALID_BODY), None);
        assert_eq!(
            cut_short_remark(r#"{"elements":[],"remark":"runtime remark: Timeout is 4"}"#),
            None,
            "bug: an informational remark is not a stopped query"
        );
        assert_eq!(cut_short_remark("<html>remark</html>"), None);
    }

    /// An Overpass answer the server cut short: a 200 whose elements are the
    /// part it found before its `[timeout:N]` ran out.
    const TIMED_OUT_BODY: &str = r#"{"version":0.6,"elements":[{"type":"way","id":1}],"remark":"runtime error: Query timed out in \"query\" at line 1 after 4 seconds."}"#;

    /// An Overpass answer the server cut short when its memory ran out.
    const OUT_OF_MEMORY_BODY: &str = r#"{"version":0.6,"elements":[],"remark":"runtime error: Query run out of memory using about 2048 MB of RAM."}"#;

    /// A complete answer with two elements, told apart from a partial one.
    const TWO_ELEMENTS_BODY: &str = r#"{"elements":[{"type":"way","id":1},{"type":"way","id":2}]}"#;

    // An instance whose query ran out of time answers 200 with what it found
    // so far and a runtime-error remark. That partial list is the instance's
    // failure, never the area's routes: the walk asks the next instance.
    #[tokio::test]
    async fn a_partial_answer_the_server_cut_short_falls_through_to_the_next_instance() {
        let cut_short = StubInstance::start(vec![Reply::Respond(200, TIMED_OUT_BODY)]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, TWO_ELEMENTS_BODY)]).await;

        let elements = fetch(&service(&[&cut_short, &healthy], FAST))
            .await
            .expect("bug: the complete second answer is used");

        assert_eq!(
            elements, 2,
            "bug: the partial answer of a timed-out query was used"
        );
        assert_eq!(cut_short.hits(), 1, "bug: a cut-short query is not retried");
        assert_eq!(healthy.hits(), 1);
    }

    // Every instance cutting its query short is the route search being
    // unavailable, never an empty area.
    #[tokio::test]
    async fn every_instance_cutting_its_query_short_is_the_unavailable_error() {
        let timed_out = StubInstance::start(vec![Reply::Respond(200, TIMED_OUT_BODY)]).await;
        let out_of_memory =
            StubInstance::start(vec![Reply::Respond(200, OUT_OF_MEMORY_BODY)]).await;

        let err = fetch(&service(&[&timed_out, &out_of_memory], FAST))
            .await
            .expect_err("bug: answers cut short by the server are not routes");

        assert_eq!(
            err.code,
            ErrorCode::ExternalServiceUnavailable,
            "bug: cut-short answers are the unavailable path, not no routes"
        );
        assert_athlete_facing(&err.to_string());
    }

    /// The bound carnet#1665 sets on the whole Overpass phase of one
    /// `discover_routes` call: "a few seconds, about 8 s". Production timing
    /// has to fit it, with room for a loaded runner's scheduling.
    const FAIL_FAST_CEILING: Duration = Duration::from_secs(10);

    /// The walk under production timing, cut at the ceiling so a walk that
    /// overruns it fails the test instead of holding it for a minute.
    async fn production_walk(endpoints: &[&StubInstance]) -> (Option<AppResult<usize>>, Duration) {
        let client = service(endpoints, OverpassTiming::PRODUCTION);
        let started = Instant::now();
        let outcome = time::timeout(FAIL_FAST_CEILING, fetch(&client)).await.ok();
        (outcome, started.elapsed())
    }

    // carnet#1665: on 2026-11-19 a hung main instance held the walk for 38 s
    // before the next was asked. A hung first instance must leave the next
    // one its attempt inside the ceiling.
    #[tokio::test]
    async fn a_hung_instance_leaves_the_next_its_attempt_within_the_fail_fast_ceiling() {
        let hung = StubInstance::start(vec![Reply::Hang]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;

        let (outcome, elapsed) = production_walk(&[&hung, &healthy]).await;

        let elements = outcome
            .unwrap_or_else(|| panic!("bug: the walk overran {FAIL_FAST_CEILING:?}"))
            .expect("bug: the healthy second instance answers once the hung one is abandoned");
        assert_eq!(
            elements, 1,
            "bug: the healthy instance's answer is the one used"
        );
        assert!(
            elapsed < FAIL_FAST_CEILING,
            "bug: a hung instance held the walk for {elapsed:?}"
        );
        assert_eq!(healthy.hits(), 1, "bug: the second instance is asked once");
    }

    // carnet#1665: every instance hung cost 78 s on 2026-11-19. The walk
    // gives up within the ceiling, with the reason the agent reads.
    #[tokio::test]
    async fn every_instance_hung_fails_within_the_fail_fast_ceiling() {
        let first = StubInstance::start(vec![Reply::Hang]).await;
        let second = StubInstance::start(vec![Reply::Hang]).await;
        let third = StubInstance::start(vec![Reply::Hang]).await;

        let (outcome, elapsed) = production_walk(&[&first, &second, &third]).await;

        let err = outcome
            .unwrap_or_else(|| panic!("bug: the walk overran {FAIL_FAST_CEILING:?}"))
            .expect_err("bug: no instance answered, so the walk fails");
        assert_eq!(
            err.code,
            ErrorCode::ExternalServiceUnavailable,
            "bug: an unanswered walk is the unavailable error"
        );
        assert!(
            elapsed < FAIL_FAST_CEILING,
            "bug: the walk took {elapsed:?}"
        );
        assert_athlete_facing(&err.to_string());
    }

    // 2026-11-27: overpass-api.de and overpass.private.coffee hung while
    // maps.mail.ru answered in 5-7 s. The walk gave each instance 5 s in
    // turn, so the instance that answered was never asked. Once a failure
    // has been seen (the first walk of an outage on a calm process), the
    // walk hedges at 1 s, and the slow third instance is asked at about 2 s
    // and its answer used.
    #[tokio::test]
    async fn a_slow_third_instance_answers_while_the_first_two_hang() {
        let first = StubInstance::start(vec![Reply::Hang]).await;
        let second = StubInstance::start(vec![Reply::Hang]).await;
        let slow = StubInstance::start(vec![Reply::After(5_000, 200, VALID_BODY)]).await;
        let failed = StubInstance::start(vec![Reply::Hang]).await;
        start_cooldown(&failed.endpoint, OVERPASS_COOLDOWN);

        let (outcome, elapsed) = production_walk(&[&first, &second, &slow, &failed]).await;

        let elements = outcome
            .unwrap_or_else(|| panic!("bug: the walk overran {FAIL_FAST_CEILING:?}"))
            .expect("bug: the slow third instance answered within the budget");
        assert_eq!(elements, 1, "bug: the slow instance's answer is used");
        assert!(
            elapsed < OVERPASS_TOTAL_BUDGET,
            "bug: the walk outlasted its budget: {elapsed:?}"
        );
        assert_eq!(
            (first.hits(), second.hits(), slow.hits(), failed.hits()),
            (1, 1, 1, 1),
            "bug: each instance is asked once, the cooling one last"
        );
    }

    // A hedge, not a race: an instance answering within the hedge delay is
    // the only one asked.
    #[tokio::test]
    async fn a_prompt_first_answer_asks_no_other_instance() {
        let prompt = StubInstance::start(vec![Reply::After(50, 200, VALID_BODY)]).await;
        let spare = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;

        fetch(&service(&[&prompt, &spare], FAST)).await.unwrap();

        assert_eq!(prompt.hits(), 1);
        assert_eq!(spare.hits(), 0, "bug: a prompt answer still hedged");
    }

    // A failure, unlike silence, asks the next instance at once: with a hedge
    // as long as the budget, the walk would otherwise wait it out.
    #[tokio::test]
    async fn a_failure_asks_the_next_instance_without_waiting_for_the_hedge() {
        let refused = StubInstance::start(vec![Reply::Respond(400, "bad query")]).await;
        let healthy = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let timing = OverpassTiming {
            hedge_delay: Duration::from_secs(5),
            calm_hedge_delay: Duration::from_secs(5),
            ..FAST
        };

        let started = Instant::now();
        let elements = fetch(&service(&[&refused, &healthy], timing))
            .await
            .expect("bug: the instance after a refusal answers");
        let elapsed = started.elapsed();

        assert_eq!(elements, 1, "bug: the healthy instance's answer is used");
        assert!(
            elapsed < Duration::from_secs(2),
            "bug: a failed instance waited out the hedge: {elapsed:?}"
        );
        assert_eq!(
            (refused.hits(), healthy.hits()),
            (1, 1),
            "bug: each instance is asked once"
        );
    }

    // The first complete answer wins, not the first instance asked: a hedged
    // instance answering while the first is still silent is the one used,
    // and the silent first one is cooled down for the next call.
    #[tokio::test]
    async fn the_first_complete_answer_wins_and_the_silent_loser_cools_down() {
        let slow = StubInstance::start(vec![Reply::After(1_500, 200, TWO_ELEMENTS_BODY)]).await;
        let quick = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let service = service(&[&slow, &quick], FAST);

        let elements = fetch(&service)
            .await
            .expect("bug: the hedged answer is used");
        assert_eq!(
            elements, 1,
            "bug: the walk waited for the first instance instead of the first answer"
        );
        assert_eq!(
            (slow.hits(), quick.hits()),
            (1, 1),
            "bug: each instance is asked once"
        );

        fetch(&service).await.expect("bug: the second call answers");
        assert_eq!(
            (slow.hits(), quick.hits()),
            (1, 2),
            "bug: the instance still silent when another answered was asked first again"
        );
    }

    // A calm day: no instance failed lately, so the main instance gets the
    // calm hedge to answer alone and the mirrors are not asked.
    #[tokio::test]
    async fn a_calm_walk_lets_the_first_instance_answer_alone() {
        let main = StubInstance::start(vec![Reply::After(400, 200, VALID_BODY)]).await;
        let mirror = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let timing = OverpassTiming {
            hedge_delay: Duration::from_millis(100),
            calm_hedge_delay: Duration::from_secs(2),
            ..FAST
        };

        fetch(&service(&[&main, &mirror], timing)).await.unwrap();

        assert_eq!(
            (main.hits(), mirror.hits()),
            (1, 0),
            "bug: a calm walk asked a mirror while the main instance was answering"
        );
    }

    // Once an instance of the client has failed, the walk hedges at the
    // short delay: the mirror is asked while the main instance is silent.
    #[tokio::test]
    async fn after_a_failure_the_walk_hedges_sooner() {
        let main = StubInstance::start(vec![Reply::After(1_500, 200, TWO_ELEMENTS_BODY)]).await;
        let mirror = StubInstance::start(vec![Reply::Respond(200, VALID_BODY)]).await;
        let failed = StubInstance::start(vec![Reply::Hang]).await;
        let timing = OverpassTiming {
            hedge_delay: Duration::from_millis(100),
            calm_hedge_delay: Duration::from_secs(4),
            ..FAST
        };
        // A failure of the client's third instance moments ago.
        start_cooldown(&failed.endpoint, timing.cooldown);

        let started = Instant::now();
        let elements = fetch(&service(&[&main, &mirror, &failed], timing))
            .await
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(elements, 1, "bug: the mirror's answer is used");
        assert!(
            elapsed < Duration::from_secs(1),
            "bug: a walk after a failure waited out the calm hedge: {elapsed:?}"
        );
        assert_eq!(failed.hits(), 0, "bug: the cooling instance was asked");
    }
}
