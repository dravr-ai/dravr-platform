// ABOUTME: Where the quota router keeps its readings and penalties: Redis when the deployment has one, this process otherwise
// ABOUTME: Keyed by the credentials the router spends, so every instance spending one budget reads and writes one view
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The quota router's state store.
//!
//! The router's readings and penalties describe an **account** — the Claude
//! subscription and the Copilot token it spends — while Cloud Run runs up to
//! `backend_max_instances` copies of the server. With per-process state each
//! instance spends its own share of one budget before noticing, steps aside
//! on its own schedule, and learns a quota refusal once per instance. When the
//! deployment has a Redis (`REDIS_URL`, the variable the platform cache reads)
//! the router shares embacle's `RedisQuotaStore` instead, keyed by the
//! fingerprints of the credentials it spends, so every instance spending that
//! budget routes on one view.
//!
//! Without a Redis — local runs and tests — the router keeps embacle's
//! in-memory store, which is the right store for a single process. A Redis
//! that cannot be reached at boot degrades to the same in-memory store with a
//! warning: shared quota state saves turns, it never gates serving them.

use std::env;
use std::time::Duration;

use embacle::quota_store::{InMemoryQuotaStore, QuotaStore};
use embacle::quota_store_redis::RedisQuotaStore;
use embacle::CliRunnerType;
use pierre_core::models::sha256_hex;
use pierre_core::redaction::redact_url;
use tokio::time::timeout;
use tracing::{info, warn};

use crate::embacle_provider::cli_credential_env_keys;

/// The variable naming the deployment's Redis — the same one the platform
/// cache's configuration reads, so one provisioned Redis serves both.
const REDIS_URL_ENV: &str = "REDIS_URL";

/// The longest boot waits for the first Redis connection before routing on
/// this process's own view. A Redis in the same VPC answers in milliseconds;
/// one that has not answered in this long is down, and the server must still
/// come up.
const CONNECT_DEADLINE: Duration = Duration::from_secs(3);

/// Hex characters of a credential's SHA-256 used in the account key — the
/// platform's secret-fingerprint width: enough to tell two credentials apart,
/// never enough to recover one, and safe to log.
const FINGERPRINT_LEN: usize = 8;

/// The router's store, resolved from the process environment: the Redis
/// `REDIS_URL` names, keyed by the credentials `runners` spend, or the
/// in-memory store when there is none.
///
/// `runners` are the router's backends in the order the router holds them;
/// the caller that builds the router passes them, so the store's keys cannot
/// drift from the backends they describe.
pub(crate) async fn quota_store_from_env(runners: &[CliRunnerType]) -> Box<dyn QuotaStore> {
    let backend_keys: Vec<String> = runners.iter().map(ToString::to_string).collect();
    let backend_keys: Vec<&str> = backend_keys.iter().map(String::as_str).collect();
    quota_store(
        redis_url_from_env().as_deref(),
        &account_from_env(runners),
        &backend_keys,
    )
    .await
}

/// The deployment's Redis URL, when one is configured.
fn redis_url_from_env() -> Option<String> {
    env::var(REDIS_URL_ENV)
        .ok()
        .filter(|url| !url.trim().is_empty())
}

/// The credential `runner` spends: the first of its credential variables
/// that is set.
fn spent_credential(runner: CliRunnerType) -> Option<String> {
    cli_credential_env_keys(runner)
        .iter()
        .find_map(|key| env::var(key).ok().filter(|v| !v.is_empty()))
}

/// The account the router's state belongs to, from the process environment.
fn account_from_env(runners: &[CliRunnerType]) -> String {
    let credentials: Vec<(String, Option<String>)> = runners
        .iter()
        .map(|runner| (runner.to_string(), spent_credential(*runner)))
        .collect();
    account(
        &credentials
            .iter()
            .map(|(name, credential)| (name.as_str(), credential.as_deref()))
            .collect::<Vec<_>>(),
    )
}

/// The account key for a router spending `credentials`, one `(runner,
/// credential)` pair per backend.
///
/// The budget being shared is the pair of credentials, so the key is both
/// fingerprints: instances spending the same pair share state, and a rotated
/// or different credential starts a namespace of its own instead of reading
/// another budget's penalties. A runner with no credential set (Claude Code
/// on a stored login) is `none`. Never the instance and never a tenant —
/// the router is a process singleton spending the platform's credentials.
fn account(credentials: &[(&str, Option<&str>)]) -> String {
    credentials
        .iter()
        .map(|(runner, credential)| {
            let fingerprint = credential.map_or_else(
                || "none".to_owned(),
                |secret| sha256_hex(secret).chars().take(FINGERPRINT_LEN).collect(),
            );
            format!("{runner}-{fingerprint}")
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// The store the router keeps its state in.
///
/// `RedisQuotaStore` when `redis_url` is set and answers within
/// [`CONNECT_DEADLINE`]; otherwise the in-memory store, with the reason
/// logged. `backend_keys` names the router's backends in order, so instances
/// that order them differently during a rollout still share each backend's
/// state.
async fn quota_store(
    redis_url: Option<&str>,
    account: &str,
    backend_keys: &[&str],
) -> Box<dyn QuotaStore> {
    let Some(url) = redis_url else {
        info!(
            store = "in-memory",
            "Quota router state is per-process: {REDIS_URL_ENV} is unset"
        );
        return Box::new(InMemoryQuotaStore::new());
    };
    match shared_store(url, account, backend_keys).await {
        Some(store) => Box::new(store),
        None => Box::new(InMemoryQuotaStore::new()),
    }
}

/// The Redis store at `url`, or `None` — with the reason logged — when it
/// cannot be reached within [`CONNECT_DEADLINE`].
async fn shared_store(url: &str, account: &str, backend_keys: &[&str]) -> Option<RedisQuotaStore> {
    let redacted = redact_url(url);
    match timeout(CONNECT_DEADLINE, RedisQuotaStore::connect(url, account)).await {
        Ok(Ok(store)) => {
            let store = store.with_backend_keys(backend_keys.iter().copied());
            info!(
                store = store.name(),
                redis = %redacted,
                account = %account,
                "Quota router state is shared through Redis across every instance spending this account"
            );
            Some(store)
        }
        Ok(Err(e)) => {
            warn!(
                store = "in-memory",
                redis = %redacted,
                error = %e.message,
                "Quota router cannot reach Redis at boot; state stays per-process"
            );
            None
        }
        Err(_) => {
            warn!(
                store = "in-memory",
                redis = %redacted,
                deadline_secs = CONNECT_DEADLINE.as_secs(),
                "Quota router's Redis did not answer at boot; state stays per-process"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    const KEYS: [&str; 2] = ["claude_code", "copilot_headless"];

    /// Reads one RESP command (`*N` then N bulk strings) off `reader`;
    /// `false` once the client hangs up.
    async fn read_command(reader: &mut BufReader<TcpStream>) -> bool {
        let mut header = String::new();
        if reader.read_line(&mut header).await.unwrap_or(0) == 0 {
            return false;
        }
        let args: usize = header.trim_start_matches('*').trim().parse().unwrap_or(0);
        for _ in 0..args {
            let mut len_line = String::new();
            if reader.read_line(&mut len_line).await.unwrap_or(0) == 0 {
                return false;
            }
            let len: usize = len_line.trim_start_matches('$').trim().parse().unwrap_or(0);
            let mut payload = vec![0_u8; len + 2];
            if reader.read_exact(&mut payload).await.is_err() {
                return false;
            }
        }
        true
    }

    /// A loopback server that answers `+OK` to every RESP command — enough
    /// for a client to complete its connection handshake.
    async fn answering_redis() -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut reader = BufReader::new(socket);
                    while read_command(&mut reader).await {
                        if reader.get_mut().write_all(b"+OK\r\n").await.is_err() {
                            break;
                        }
                    }
                });
            }
        });
        format!("redis://{addr}")
    }

    #[tokio::test]
    async fn a_reachable_redis_shares_the_router_state() {
        let url = answering_redis().await;
        let store = quota_store(Some(&url), "acct", &KEYS).await;
        assert_eq!(
            store.name(),
            "redis",
            "a Redis that answers must carry the router's state, not this process"
        );
    }

    #[tokio::test]
    async fn no_redis_configured_keeps_the_in_memory_store() {
        let store = quota_store(None, "acct", &KEYS).await;
        assert_eq!(store.name(), "in-memory");
    }

    #[tokio::test]
    async fn an_unreachable_redis_degrades_to_the_in_memory_store_without_holding_boot() {
        // Port 1 on loopback refuses: the configured branch is taken, the
        // connection fails, and boot carries on per-process.
        let started = Instant::now();
        let store = quota_store(Some("redis://127.0.0.1:1"), "acct", &KEYS).await;
        assert_eq!(store.name(), "in-memory");
        assert!(
            started.elapsed() <= CONNECT_DEADLINE + Duration::from_secs(1),
            "an unreachable Redis must not hold boot past the connect deadline"
        );
    }

    #[tokio::test]
    async fn a_malformed_redis_url_degrades_to_the_in_memory_store() {
        let store = quota_store(Some("not a url"), "acct", &KEYS).await;
        assert_eq!(store.name(), "in-memory");
    }

    #[test]
    fn the_account_is_the_pair_of_credentials_never_the_credentials_themselves() {
        let key = account(&[
            ("claude_code", Some("sk-ant-oat-secret")),
            ("copilot_headless", Some("ghp_secret")),
        ]);
        assert!(!key.contains("secret"), "a credential leaked into {key}");
        let parts: Vec<&str> = key.split('.').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("claude_code-"));
        assert!(parts[1].starts_with("copilot_headless-"));
        assert_eq!(parts[0].len(), "claude_code-".len() + FINGERPRINT_LEN);
    }

    #[test]
    fn the_same_credentials_name_the_same_account_and_a_different_one_does_not() {
        let same =
            |claude, copilot| account(&[("claude_code", claude), ("copilot_headless", copilot)]);
        assert_eq!(same(Some("a"), Some("b")), same(Some("a"), Some("b")));
        assert_ne!(same(Some("a"), Some("b")), same(Some("a"), Some("c")));
        assert_ne!(same(Some("a"), Some("b")), same(Some("z"), Some("b")));
    }

    #[test]
    fn a_runner_on_a_stored_login_is_named_none() {
        let key = account(&[("claude_code", None), ("copilot_headless", Some("ghp"))]);
        assert!(key.starts_with("claude_code-none."));
    }
}
