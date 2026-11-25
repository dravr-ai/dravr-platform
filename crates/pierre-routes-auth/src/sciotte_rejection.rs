// ABOUTME: The WARN line a sign-in the provider turned down writes, which the ops Slack alert matches
// ABOUTME: Names the provider, its reason and the sign-in method, with the account masked whatever its shape
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_middleware::redaction;
use tracing::warn;
use uuid::Uuid;

use crate::sciotte::LoginStep;

/// Log a sign-in the provider turned down (`LoginOutcome::Failed`) at WARN.
///
/// An athlete the provider turned away is stuck until someone looks:
/// `infra/environments/dev/provider_sign_in_monitoring.tf` matches this line
/// on its target and message, and posts every field on it to ops Slack. The
/// target is pinned so the match holds wherever the line lives. The account
/// is masked ([`masked_account`]), and so is any address in the provider's
/// reason, which is text read off its page and may quote the account back.
pub(crate) fn log_sign_in_rejected(step: LoginStep<'_>, user_id: Uuid, reason: &str) {
    let (provider, method_used, account) = match step {
        LoginStep::Login {
            provider,
            method,
            account,
            ..
        } => (provider, method.as_str(), masked_account(account)),
        // A code or a 2FA pick continues a held flow, which keeps no account.
        LoginStep::Continuation { provider, .. } => (provider, "continuation", String::new()),
    };
    warn!(
        target: "pierre_routes_auth::sciotte",
        user_id = %user_id,
        provider = %provider,
        reason = %redaction::mask_email(reason),
        method_used = %method_used,
        account = %account,
        "Sciotte remote login rejected by provider (user credentials)"
    );
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
    use std::fs;
    use std::path::Path;

    use dravr_sciotte::wire::LoginMethod;
    use pierre_test_support::log_capture::{self, CapturedEvent};
    use tracing::Level;

    use super::*;

    fn rejected(step: LoginStep<'_>, reason: &str) -> CapturedEvent {
        let ((), events) = log_capture::capture_events_while(|| {
            log_sign_in_rejected(step, Uuid::from_u128(7), reason);
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

    #[test]
    fn a_refusal_is_a_warn_naming_the_provider_method_and_a_masked_address() {
        let event = rejected(
            login("jane.doe@example.com"),
            "No account for jane.doe@example.com",
        );
        assert_eq!(event.level, Level::WARN, "the alert reads a WARN line");
        assert_eq!(event.field("provider"), Some("sciotte_strava"));
        assert_eq!(event.field("method_used"), Some("google"), "the wire name");
        assert_eq!(event.field("account"), Some("j***@e***.com"));
        assert_eq!(
            event.field("reason"),
            Some("No account for j***@e***.com"),
            "an address the provider quotes back is masked too"
        );
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
    fn a_refused_continuation_names_no_account() {
        let event = rejected(
            LoginStep::Continuation {
                provider: "sciotte_garmin",
                flow_id: "flow-1",
            },
            "Code expired",
        );
        assert_eq!(event.field("method_used"), Some("continuation"));
        assert_eq!(event.field("account"), Some(""));
        assert_eq!(event.field("provider"), Some("sciotte_garmin"));
    }

    // A parity check between the log line and the Cloud Monitoring alert that
    // matches it: nothing at runtime joins the two, so the alert's filter and
    // label extractors are read from the Terraform and compared with the
    // event this module emits.
    #[test]
    fn the_ops_alert_matches_the_line_and_extracts_every_field_on_it() {
        let tf_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infra/environments/dev/provider_sign_in_monitoring.tf");
        let tf = fs::read_to_string(&tf_path)
            .unwrap_or_else(|e| panic!("the alert lives at {}: {e}", tf_path.display()));
        let event = rejected(login("jane.doe@example.com"), "Invalid password");

        assert!(
            tf.contains(&format!("labels.\"rust.target\"=\"{}\"", event.target)),
            "the alert filters on the line's target {}",
            event.target
        );
        let matched = tf
            .split("jsonPayload.message:\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the alert matches on the message text");
        let message = event.message().expect("the line has a message");
        assert!(
            message.contains(matched),
            "the alert's {matched:?} matches the line {message:?}"
        );
        for field in event.fields.keys().filter(|key| *key != "message") {
            assert!(
                tf.contains(&format!("EXTRACT(jsonPayload.{field})")),
                "the alert extracts the line's {field} field"
            );
        }
    }
}
