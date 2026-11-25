// ABOUTME: The resource servers' one DPoP rule set (RFC 9449 §7), applied to a JWT whose signature and claims verified
// ABOUTME: A bound token needs the DPoP scheme and a proof by its key; the DPoP scheme needs a bound token; Bearer stays as before

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::auth::Claims;
use pierre_auth::dpop::{check_binding, dpop_credential, DpopContext, DpopRefusal, PresentedScheme};
use pierre_core::errors::AppError;
use tracing::warn;

use super::outcome::Refused;
use super::McpAuthMiddleware;

/// How a JWT reached the middleware: the scheme it was presented under, and
/// the request's `DPoP` context on a path that takes proofs (`/mcp`, A2A).
#[derive(Debug, Clone, Copy)]
pub(super) struct Presented<'a> {
    pub(super) scheme: PresentedScheme,
    pub(super) dpop: Option<&'a DpopContext<'a>>,
}

impl Presented<'_> {
    /// A cookie, or a header on a path that takes no proof: Bearer, no proof.
    pub(super) const BEARER: Presented<'static> = Presented {
        scheme: PresentedScheme::Bearer,
        dpop: None,
    };
}

/// The JWT an `Authorization` value carries and the scheme it names:
/// `Bearer <jwt>` or `DPoP <jwt>`; `None` for any other value.
pub(super) fn jwt_credential(header: &str) -> Option<(&str, PresentedScheme)> {
    header
        .strip_prefix("Bearer ")
        .map(|token| (token, PresentedScheme::Bearer))
        .or_else(|| dpop_credential(header).map(|token| (token, PresentedScheme::Dpop)))
}

impl McpAuthMiddleware {
    /// Apply the binding rules to `claims`, verified from `token`, before the
    /// request spends anything. Without a configured proof verifier, the rules
    /// that need no proof still hold and any bound token is refused, so a
    /// server missing its replay memory refuses rather than admits.
    pub(super) async fn require_dpop_binding(
        &self,
        claims: &Claims,
        token: &str,
        presented: Presented<'_>,
    ) -> Result<(), Refused> {
        let outcome = match &self.dpop {
            Some(proofs) => {
                proofs
                    .check_presentation(claims, token, presented.scheme, presented.dpop)
                    .await
            }
            None => check_binding(claims, presented.scheme).and_then(|()| {
                if claims.cnf.is_some() {
                    Err(DpopRefusal::Missing)
                } else {
                    Ok(())
                }
            }),
        };
        outcome.map_err(|refusal| {
            warn!(refusal = %refusal, "Request refused by the DPoP binding rules");
            let error = if refusal.is_server_fault() {
                AppError::internal(refusal.to_string())
            } else {
                AppError::auth_invalid(format!("DPoP: {refusal}"))
            };
            Refused::Dpop {
                error,
                code: refusal.error_code(),
            }
        })
    }
}
