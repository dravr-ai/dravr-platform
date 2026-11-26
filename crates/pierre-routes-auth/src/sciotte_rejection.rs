// ABOUTME: What a sign-in the provider turned down leaves behind: a tagged WARN, the window's count, a queued follow-up
// ABOUTME: Names the provider, its reason and the sign-in method, with the account masked whatever its shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use dravr_tronc::dates::Timestamp;
use pierre_cache::Cache;
use pierre_middleware::redaction;
use tracing::{info, warn};
use uuid::Uuid;

use crate::sciotte::LoginStep;
use crate::sign_in_follow_up::{
    last_refusal_key, refusals_key, FollowUp, LastRefusal, SignInFollowUps, REFUSAL_WINDOW,
};

/// Record a sign-in the provider turned down (`LoginOutcome::Failed`).
///
/// It counts the refusal in the athlete's refusal window, logs it at WARN
/// with its rank there, keeps it as their last refusal, and queues the
/// follow-up that checks, ten minutes on, whether they connected anyway.
///
/// None of it can fail the sign-in's response: a cache that cannot count
/// logs ordinal 0, and a record or a follow-up that cannot be kept is a WARN
/// of its own.
pub(crate) async fn sign_in_refused(
    cache: &Cache,
    follow_ups: Option<&Arc<dyn SignInFollowUps>>,
    step: LoginStep<'_>,
    (user_id, tenant_id): (Uuid, Uuid),
    reason: &str,
) {
    let provider = step.provider();
    let ordinal = count_refusal(cache, tenant_id, user_id, provider).await;
    log_sign_in_rejected(step, user_id, reason, ordinal);

    let (method_used, account) = method_and_account(step);
    let key = last_refusal_key(tenant_id, user_id, provider);
    // A code or a 2FA pick keeps no account: the record keeps the one the
    // login it continues named, so the alert still says which account.
    let account = if account.is_empty() {
        cache
            .get::<LastRefusal>(&key)
            .await
            .ok()
            .flatten()
            .map_or_else(String::new, |previous| previous.account)
    } else {
        account
    };
    let last = LastRefusal {
        refused_at_ms: Timestamp::now().epoch_millis(),
        reason: redaction::mask_email(reason),
        method_used: method_used.to_owned(),
        account,
        refusals: ordinal,
    };
    if let Err(e) = cache.set(&key, &last, REFUSAL_WINDOW).await {
        // With no record, the follow-up would find nothing to follow.
        warn!(user_id = %user_id, provider = %provider, error = %e, "Could not keep the sign-in refusal; no follow-up is queued for it");
        return;
    }
    let Some(follow_ups) = follow_ups else {
        return;
    };
    let due = FollowUp {
        tenant_id,
        user_id,
        backend: provider.to_owned(),
        refused_at_ms: last.refused_at_ms,
    };
    if let Err(e) = follow_ups.queue(due).await {
        warn!(user_id = %user_id, provider = %provider, error = %e, "Could not queue the sign-in refusal's follow-up");
    }
}

/// Count one refusal of `user_id`'s sign-ins to `provider` in its window, and
/// return its rank there; 0 when the cache cannot count.
async fn count_refusal(cache: &Cache, tenant_id: Uuid, user_id: Uuid, provider: &str) -> u64 {
    let key = refusals_key(tenant_id, user_id, provider);
    match cache.count_in_window(&key, REFUSAL_WINDOW).await {
        Ok(count) => count.hits,
        Err(e) => {
            warn!(user_id = %user_id, provider = %provider, error = %e, "Could not count the sign-in refusal; it is logged as ordinal 0");
            0
        }
    }
}

/// The refusals `user_id`'s sign-ins to `provider` met before this one.
///
/// Read without counting: the window's count, or the last refusal's rank
/// once the window closed; 0 when neither is there or the cache cannot say.
async fn prior_refusals(cache: &Cache, tenant_id: Uuid, user_id: Uuid, provider: &str) -> u64 {
    match cache
        .get::<u64>(&refusals_key(tenant_id, user_id, provider))
        .await
    {
        Ok(Some(count)) => return count,
        Ok(None) => {}
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Could not read the sign-in refusal window");
        }
    }
    match cache
        .get::<LastRefusal>(&last_refusal_key(tenant_id, user_id, provider))
        .await
    {
        Ok(last) => last.map_or(0, |last| last.refusals),
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Could not read the last sign-in refusal");
            0
        }
    }
}

/// Log a sign-in the service authenticated, with the refusals it came
/// after and the window's attempt.
pub(crate) async fn log_sign_in_succeeded(
    cache: &Cache,
    user_id: Uuid,
    tenant_id: Uuid,
    provider: &str,
    attempt_id: &str,
) {
    let prior_refusals = prior_refusals(cache, tenant_id, user_id, provider).await;
    info!(
        target: "pierre_routes_auth::sciotte",
        user_id = %user_id,
        provider = %provider,
        prior_refusals,
        attempt_id = %attempt_id,
        "Session sign-in successful (remote service)"
    );
}

/// Log a sign-in the provider turned down, at WARN.
///
/// The line is tagged with the window's attempt and the refusal's rank in
/// the athlete's refusal window.
///
/// The target is pinned so a search for the line holds wherever it lives.
/// The account is masked ([`masked_account`]), and so is any address in the
/// provider's reason, which is text read off its page and may quote the
/// account back.
fn log_sign_in_rejected(step: LoginStep<'_>, user_id: Uuid, reason: &str, refusal_ordinal: u64) {
    let (method_used, account) = method_and_account(step);
    warn!(
        target: "pierre_routes_auth::sciotte",
        user_id = %user_id,
        provider = %step.provider(),
        reason = %redaction::mask_email(reason),
        method_used = %method_used,
        account = %account,
        attempt_id = %step.attempt_id(),
        refusal_ordinal,
        "Sciotte remote login rejected by provider (user credentials)"
    );
}

/// How the step signed in, and the account it typed, masked; a code or a
/// 2FA pick continues a held flow, which keeps no account.
fn method_and_account(step: LoginStep<'_>) -> (&'static str, String) {
    match step {
        LoginStep::Login {
            method, account, ..
        } => (method.as_str(), masked_account(account)),
        LoginStep::Continuation { .. } => ("continuation", String::new()),
    }
}

/// The account the athlete typed, as the ops log names it.
///
/// An address is masked by [`redaction::mask_email`]. Anything else (a phone
/// number, a username) is handed back whole by it, so it goes through
/// [`redaction::mask_recipient`] instead, which keeps the last four
/// characters only.
fn masked_account(account: &str) -> String {
    let masked = redaction::mask_email(account);
    if masked == account {
        redaction::mask_recipient(account)
    } else {
        masked
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use dravr_sciotte::wire::LoginMethod;
    use pierre_cache::CacheConfig;
    use pierre_core::errors::AppResult;
    use pierre_test_support::log_capture::{self, CapturedEvent};
    use tracing::Level;

    use super::*;
    use crate::sciotte_cancel::StepAttempt;

    const TENANT: Uuid = Uuid::from_u128(11);
    const USER: Uuid = Uuid::from_u128(7);

    fn rejected(step: LoginStep<'_>, reason: &str) -> CapturedEvent {
        let ((), events) = log_capture::capture_events_while(|| {
            log_sign_in_rejected(step, USER, reason, 1);
        });
        assert_eq!(events.len(), 1, "one line per refusal: {events:#?}");
        events.into_iter().next().unwrap() // Safe: length asserted above
    }

    fn login(account: &str) -> LoginStep<'_> {
        LoginStep::Login {
            provider: "sciotte_strava",
            attempt: None,
            method: LoginMethod::Google,
            account,
        }
    }

    async fn cache() -> Cache {
        Cache::new(CacheConfig {
            enable_background_cleanup: false,
            ..CacheConfig::default()
        })
        .await
        .unwrap() // Safe: an in-memory cache always builds
    }

    /// The follow-ups a refusal queued.
    #[derive(Debug, Default)]
    struct Queued(Mutex<Vec<FollowUp>>);

    #[async_trait]
    impl SignInFollowUps for Queued {
        async fn queue(&self, follow_up: FollowUp) -> AppResult<()> {
            self.0.lock().unwrap().push(follow_up); // Safe: never poisoned in a test
            Ok(())
        }
    }

    fn garmin_code(attempt: Option<&StepAttempt>) -> LoginStep<'_> {
        LoginStep::Continuation {
            provider: "sciotte_garmin",
            attempt,
            flow_id: "flow-1",
        }
    }

    /// Every refusal line among `events`, in order.
    fn refusal_lines(events: &[CapturedEvent]) -> Vec<&CapturedEvent> {
        events
            .iter()
            .filter(|event| {
                event.message()
                    == Some("Sciotte remote login rejected by provider (user credentials)")
            })
            .collect()
    }

    #[test]
    fn a_refusal_is_a_warn_naming_the_provider_method_and_a_masked_address() {
        let event = rejected(
            login("jane.doe@example.com"),
            "No account for jane.doe@example.com",
        );
        assert_eq!(event.level, Level::WARN);
        assert_eq!(
            event.target, "pierre_routes_auth::sciotte",
            "the target is pinned"
        );
        assert_eq!(event.field("provider"), Some("sciotte_strava"));
        assert_eq!(event.field("method_used"), Some("google"), "the wire name");
        assert_eq!(event.field("account"), Some("j***@e***.com"));
        assert_eq!(
            event.field("reason"),
            Some("No account for j***@e***.com"),
            "an address the provider quotes back is masked too"
        );
        assert_eq!(
            event.field("attempt_id"),
            Some(""),
            "no attempt named, none logged"
        );
        assert_eq!(event.field("refusal_ordinal"), Some("1"));
        let line = format!("{:?}", event.fields);
        assert!(
            !line.contains("jane.doe"),
            "no field holds the address: {line}"
        );
    }

    #[test]
    fn an_account_that_is_not_an_address_keeps_only_its_last_four_characters() {
        let phone = rejected(login("+15145550199"), "Invalid password");
        assert_eq!(phone.field("account"), Some("********0199"));

        let username = rejected(login("marathoner"), "Invalid password");
        assert_eq!(username.field("account"), Some("******oner"));

        // mask_email leaves an address this short as it is.
        let short = rejected(login("a@b.co"), "Invalid password");
        assert_eq!(short.field("account"), Some("**b.co"));
    }

    #[test]
    fn a_refused_continuation_names_no_account_and_its_attempt() {
        let attempt = StepAttempt::of_step(TENANT, USER, Some("attempt-0001"))
            .unwrap() // Safe: a well-formed id
            .unwrap(); // Safe: an id was named
        let event = rejected(garmin_code(Some(&attempt)), "Code expired");
        assert_eq!(event.field("method_used"), Some("continuation"));
        assert_eq!(event.field("account"), Some(""));
        assert_eq!(event.field("provider"), Some("sciotte_garmin"));
        assert_eq!(event.field("attempt_id"), Some("attempt-0001"));
    }

    #[tokio::test]
    async fn refusals_rank_in_one_window_across_both_backends_and_each_queues_its_follow_up() {
        let cache = cache().await;
        let queued = Arc::new(Queued::default());
        let follow_ups: Arc<dyn SignInFollowUps> = queued.clone();
        let (log, _guard) = log_capture::capture_events();

        sign_in_refused(
            &cache,
            Some(&follow_ups),
            garmin_code(None),
            (USER, TENANT),
            "Code expired",
        )
        .await;
        let oauth_garmin = LoginStep::Login {
            provider: "garmin",
            attempt: None,
            method: LoginMethod::Email,
            account: "jane.doe@example.com",
        };
        sign_in_refused(
            &cache,
            Some(&follow_ups),
            oauth_garmin,
            (USER, TENANT),
            "Bad password for jane.doe@example.com",
        )
        .await;

        let events = log.events();
        let lines = refusal_lines(&events);
        let ordinals: Vec<_> = lines.iter().map(|e| e.field("refusal_ordinal")).collect();
        assert_eq!(
            ordinals,
            [Some("1"), Some("2")],
            "bug: garmin and sciotte_garmin count apart"
        );

        let last: LastRefusal = cache
            .get(&last_refusal_key(TENANT, USER, "garmin"))
            .await
            .unwrap() // Safe: in-memory reads do not fail
            .expect("the last refusal is kept");
        assert_eq!(last.refusals, 2);
        assert_eq!(last.method_used, "email");
        assert_eq!(
            last.account, "j***@e***.com",
            "the record holds the account masked"
        );
        assert_eq!(
            last.reason, "Bad password for j***@e***.com",
            "and the reason masked"
        );

        let queued = queued.0.lock().unwrap().clone(); // Safe: never poisoned in a test
        assert_eq!(queued.len(), 2, "one follow-up per refusal");
        assert_eq!(
            queued[1].refused_at_ms, last.refused_at_ms,
            "the last follow-up names the last refusal"
        );
        assert_eq!(queued[1].backend, "garmin");
        assert_eq!((queued[1].tenant_id, queued[1].user_id), (TENANT, USER));
    }

    #[tokio::test]
    async fn a_refused_code_keeps_the_account_the_refused_login_before_it_named() {
        let cache = cache().await;
        let login = LoginStep::Login {
            provider: "sciotte_garmin",
            attempt: None,
            method: LoginMethod::Email,
            account: "jane.doe@example.com",
        };
        sign_in_refused(&cache, None, login, (USER, TENANT), "Invalid password").await;
        sign_in_refused(
            &cache,
            None,
            garmin_code(None),
            (USER, TENANT),
            "Code expired",
        )
        .await;

        let last: LastRefusal = cache
            .get(&last_refusal_key(TENANT, USER, "garmin"))
            .await
            .unwrap() // Safe: in-memory reads do not fail
            .expect("the last refusal is kept");
        assert_eq!(last.method_used, "continuation");
        assert_eq!(last.reason, "Code expired");
        assert_eq!(
            last.account, "j***@e***.com",
            "bug: a refused code erased the account the alert names"
        );
        assert_eq!(last.refusals, 2);
    }

    #[tokio::test]
    async fn a_cache_that_cannot_count_logs_ordinal_zero_and_the_refusal_still_answers() {
        let cache = cache().await;
        // A value that is no window counter: count_in_window refuses it.
        cache
            .set(
                &refusals_key(TENANT, USER, "garmin"),
                &"not a counter",
                REFUSAL_WINDOW,
            )
            .await
            .unwrap(); // Safe: in-memory writes do not fail
        let (log, _guard) = log_capture::capture_events();

        sign_in_refused(
            &cache,
            None,
            garmin_code(None),
            (USER, TENANT),
            "Code expired",
        )
        .await;

        let events = log.events();
        let lines = refusal_lines(&events);
        assert_eq!(lines.len(), 1, "the refusal is still logged");
        assert_eq!(lines[0].field("refusal_ordinal"), Some("0"));
    }

    #[tokio::test]
    async fn a_sign_in_logs_the_refusals_before_it_without_counting_one() {
        let cache = cache().await;
        let (log, _guard) = log_capture::capture_events();
        for _ in 0..3 {
            sign_in_refused(
                &cache,
                None,
                garmin_code(None),
                (USER, TENANT),
                "Code expired",
            )
            .await;
        }
        log_sign_in_succeeded(&cache, USER, TENANT, "sciotte_garmin", "attempt-0001").await;
        log_sign_in_succeeded(&cache, USER, TENANT, "sciotte_garmin", "attempt-0001").await;

        let events = log.events();
        let signed_in: Vec<_> = events
            .iter()
            .filter(|event| event.message() == Some("Session sign-in successful (remote service)"))
            .collect();
        assert_eq!(signed_in.len(), 2);
        for line in signed_in {
            assert_eq!(
                line.field("prior_refusals"),
                Some("3"),
                "bug: reading the window counted a hit"
            );
            assert_eq!(line.field("attempt_id"), Some("attempt-0001"));
            assert_eq!(line.target, "pierre_routes_auth::sciotte");
        }

        // Once the window is gone, the last refusal's rank still answers.
        cache
            .invalidate(&refusals_key(TENANT, USER, "garmin"))
            .await
            .unwrap(); // Safe: in-memory
        log_sign_in_succeeded(&cache, USER, TENANT, "garmin", "").await;
        let after = log.events();
        let last_line = after.last().expect("a line was logged");
        assert_eq!(last_line.field("prior_refusals"), Some("3"));
    }

    #[tokio::test]
    async fn a_first_try_sign_in_logs_no_prior_refusals() {
        let cache = cache().await;
        let (log, _guard) = log_capture::capture_events();
        log_sign_in_succeeded(&cache, USER, TENANT, "sciotte", "").await;
        let events = log.events();
        assert_eq!(
            events.last().and_then(|e| e.field("prior_refusals")),
            Some("0")
        );
    }
}
