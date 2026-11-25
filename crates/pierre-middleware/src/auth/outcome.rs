// ABOUTME: One credential's authentication outcome: admitted under its budget, or refused and whom it named
// ABOUTME: Split from auth.rs; settles an outcome into the request's answer and reports the deciding budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::auth::AuthResult;
use pierre_auth::rate_limiting::RequestBudget;
use pierre_core::errors::AppError;
use uuid::Uuid;

use crate::rate_limiting::report_request_budget;

/// A credential its request budget admitted, and that budget.
pub(super) struct Admitted {
    pub(super) auth: AuthResult,
    pub(super) budget: RequestBudget,
}

/// Why one credential did not authenticate the request.
pub(super) enum Refused {
    /// Its own request budget is spent: the 429, the budget that produced
    /// it, and the caller whose budget it is.
    OverBudget {
        budget: RequestBudget,
        error: AppError,
        caller: Uuid,
    },
    /// Refused before any budget decided: a bad credential, a delegated grant
    /// on a path that reads no scope, an owner the account-status gate turns
    /// away, a server fault. `caller` is the account the credential named,
    /// when it was checked far enough to name one.
    Failed {
        error: AppError,
        caller: Option<Uuid>,
    },
    /// Refused by the `DPoP` binding rules (RFC 9449 §7): answered with a
    /// `DPoP` challenge naming `code` beside the Bearer one.
    Dpop { error: AppError, code: &'static str },
}

impl Refused {
    pub(super) const fn error(&self) -> &AppError {
        match self {
            Self::OverBudget { error, .. }
            | Self::Failed { error, .. }
            | Self::Dpop { error, .. } => error,
        }
    }

    /// This refusal, of a credential already found to name `caller`.
    pub(super) fn of_caller(self, caller: Uuid) -> Self {
        match self {
            Self::Failed {
                error,
                caller: None,
            } => Self::Failed {
                error,
                caller: Some(caller),
            },
            named => named,
        }
    }
}

impl From<AppError> for Refused {
    fn from(error: AppError) -> Self {
        Self::Failed {
            error,
            caller: None,
        }
    }
}

/// A request authentication refused, and whom its credential named.
///
/// The caller is known when the credential was checked far enough to name
/// one: a valid API key or MCP token, a JWT whose signature holds. A door
/// that keeps an audit trail writes the refused attempt of a caller it can
/// name.
#[derive(Debug)]
pub struct AuthRefusal {
    /// The account the credential named; `None` when it named none that
    /// holds (missing, malformed, unknown, forged or expired).
    pub caller: Option<Uuid>,
    /// The refusal the request is answered with.
    pub error: AppError,
    /// The RFC 9449 error code when the `DPoP` binding rules refused the
    /// request, which a resource server answers with a `DPoP` challenge;
    /// `None` for every other refusal.
    pub dpop_error: Option<&'static str>,
}

impl From<AppError> for AuthRefusal {
    fn from(error: AppError) -> Self {
        Self {
            caller: None,
            error,
            dpop_error: None,
        }
    }
}

/// One credential checked, before its outcome is the request's answer.
pub(super) type Checked = Result<Admitted, Refused>;

/// Make one credential's outcome the request's answer, and report the budget
/// that decided it to the response's `X-RateLimit-*` headers.
///
/// The only place authentication reports a budget, so the headers name one
/// credential: the one that admitted the request, or the one whose own spent
/// budget produced the 429. A credential checked and then superseded (a spent
/// cookie falling through to the `Authorization` header) is never settled,
/// and a refusal no budget decided (a malformed token, a suspended owner, a
/// server fault) reports nothing.
pub(super) fn settle(checked: Checked) -> Result<AuthResult, AuthRefusal> {
    match checked {
        Ok(Admitted { auth, budget }) => {
            report_request_budget(budget);
            Ok(auth)
        }
        Err(Refused::OverBudget {
            budget,
            error,
            caller,
        }) => {
            report_request_budget(budget);
            Err(AuthRefusal {
                caller: Some(caller),
                error,
                dpop_error: None,
            })
        }
        Err(Refused::Failed { error, caller }) => Err(AuthRefusal {
            caller,
            error,
            dpop_error: None,
        }),
        Err(Refused::Dpop { error, code }) => Err(AuthRefusal {
            caller: None,
            error,
            dpop_error: Some(code),
        }),
    }
}
