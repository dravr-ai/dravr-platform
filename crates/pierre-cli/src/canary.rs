// ABOUTME: Credential-login canary: signs a dedicated account in through the deployed sciotte service
// ABOUTME: Classifies the first decision point so a provider-side login regression surfaces before an athlete meets it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Credential-login canary (carnet#1078).
//!
//! From about 2026-10-09 until 2026-10-16 no new athlete could connect Garmin:
//! Garmin put a captcha in front of the sign-in and sciotte waited out its
//! whole budget behind it (carnet#1035). The only signal was athletes failing.
//! This canary runs the path an athlete takes — the platform's own
//! [`SciotteClient::login_with_credentials`] against the deployed service, its
//! flags and its egress — with a dedicated account, and stops at the first
//! decision point. It never answers a code: a flow parked on one expires on the
//! service's pending-login timer, and a session the login creates is deleted.

use dravr_sciotte::client::{ClientError, SciotteClient, StatusCode};
use dravr_sciotte::wire::{LoginMethod, LoginOutcome, SESSION_EXPIRED};
use serde_json::{json, Value};
use tracing::warn;

/// Why a canary login failed at the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailReason {
    /// The provider put a challenge (a captcha) in front of the login.
    Captcha,
    /// The provider gave no answer after the password was submitted.
    Stall,
    /// The provider refused the credentials.
    Rejected,
}

impl FailReason {
    /// The reason as the verdict line names it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Captcha => "captcha",
            Self::Stall => "stall",
            Self::Rejected => "rejected",
        }
    }
}

/// What one canary login came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanaryVerdict {
    /// The provider let the login through to its next step.
    Pass {
        /// The step the login reached: `otp_required`, `two_factor_choice`,
        /// `number_match` or `authenticated`.
        step: &'static str,
    },
    /// The provider stopped the login.
    Fail {
        /// Why.
        reason: FailReason,
        /// The service's or the provider's own words, when there were any.
        detail: String,
    },
    /// The service could not be asked: nothing is known about the provider.
    Infra {
        /// What went wrong reaching the service.
        detail: String,
    },
}

impl CanaryVerdict {
    /// The verdict as the one JSON line the canary prints.
    #[must_use]
    pub fn to_json(&self, provider: &str) -> Value {
        match self {
            Self::Pass { step } => json!({"provider": provider, "verdict": "pass", "step": step}),
            Self::Fail { reason, detail } => json!({
                "provider": provider, "verdict": "fail", "reason": reason.as_str(), "detail": detail
            }),
            Self::Infra { detail } => {
                json!({"provider": provider, "verdict": "infra", "detail": detail})
            }
        }
    }
}

/// Sign `email` in to `provider` through `client` and classify the first
/// decision point.
///
/// The login is always cold (`prior_session` is `None`): a remembered device
/// would measure something an athlete connecting for the first time never
/// gets. A session the login creates is deleted before returning.
pub async fn run_login_canary(
    client: &SciotteClient,
    provider: &str,
    email: &str,
    password: &str,
) -> CanaryVerdict {
    let result = client
        .login_with_credentials(email, password, LoginMethod::Email, provider, None)
        .await;
    if let Ok(LoginOutcome::Authenticated { session_id, .. }) = &result {
        if let Err(e) = client.delete_session(session_id).await {
            warn!(error = %e, "canary could not delete the session its login created");
        }
    }
    classify(result)
}

/// The verdict a login step's result comes to.
fn classify(result: Result<LoginOutcome, ClientError>) -> CanaryVerdict {
    match result {
        Ok(LoginOutcome::OtpRequired { .. }) => CanaryVerdict::Pass {
            step: "otp_required",
        },
        Ok(LoginOutcome::TwoFactorChoice { .. }) => CanaryVerdict::Pass {
            step: "two_factor_choice",
        },
        Ok(LoginOutcome::NumberMatch { .. }) => CanaryVerdict::Pass {
            step: "number_match",
        },
        Ok(LoginOutcome::Authenticated { .. }) => CanaryVerdict::Pass {
            step: "authenticated",
        },
        Ok(LoginOutcome::Failed { reason }) => CanaryVerdict::Fail {
            reason: FailReason::Rejected,
            detail: reason,
        },
        Err(ClientError::LoginChallenge { message, .. }) => CanaryVerdict::Fail {
            reason: FailReason::Captcha,
            detail: message.unwrap_or_default(),
        },
        // The service answers `401 session_expired` on a credential login that
        // got no answer after the password was submitted (and, before sciotte
        // v0.26, on one that exhausted its whole budget). The client names
        // that marker only on a read, so on a login it arrives as `Unexpected`.
        Err(ClientError::Unexpected { status, body, .. })
            if status == StatusCode::UNAUTHORIZED
                && body.get("error").and_then(Value::as_str) == Some(SESSION_EXPIRED) =>
        {
            CanaryVerdict::Fail {
                reason: FailReason::Stall,
                detail: body
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }
        }
        Err(other) => CanaryVerdict::Infra {
            detail: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_line_names_the_provider_the_verdict_and_its_reason() {
        let fail = CanaryVerdict::Fail {
            reason: FailReason::Captcha,
            detail: "captcha in front of the sign-in".to_owned(),
        };
        assert_eq!(
            fail.to_json("garmin"),
            json!({
                "provider": "garmin", "verdict": "fail", "reason": "captcha",
                "detail": "captcha in front of the sign-in"
            })
        );
        let pass = CanaryVerdict::Pass {
            step: "number_match",
        };
        assert_eq!(
            pass.to_json("garmin"),
            json!({"provider": "garmin", "verdict": "pass", "step": "number_match"})
        );
        let infra = CanaryVerdict::Infra {
            detail: "no response".to_owned(),
        };
        assert_eq!(
            infra.to_json("garmin"),
            json!({"provider": "garmin", "verdict": "infra", "detail": "no response"})
        );
    }
}
