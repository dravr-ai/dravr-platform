// ABOUTME: A standing policy's renewal at the consent skip path: a preset grant renewed without the screen
// ABOUTME: Only for the same terms, at most T1, under the consent text the policy was approved under; the grant cites it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Renewing a preset grant under a standing policy (carnet#1544, owl §6.4)
//!
//! Read where the consent screen would be skipped, after the live-grant skip
//! misses, and inside the same gate (never for an admin, never under
//! `consent_every_time`). A request names a preset in one of two ways, and
//! both renew: a request's own details name it, and a scope-only request
//! (every deployed client sends one; the picker chooses the preset on the
//! screen) is read as asking for what the picker resolves for the policy's
//! preset, with no client constraints. A scope-only request from a client the
//! athlete holds two live policies for shows the screen: the AS never picks
//! between presets.
//!
//! A renewal needs all of:
//! - the requested details have exactly the policy's terms
//!   ([`ResolvedDetails::same_terms`]: a request that differs in any field,
//!   its purpose and claims included, shows the screen);
//! - they read at most T1;
//! - the policy's digest is the one its terms have under the consent text
//!   its language reads now for this request: a changed consent text shows
//!   the screen again.
//!
//! The AS then records the grant through the one approval function the
//! consent screen uses ([`OAuth2Routes::record_approval`]): its details
//! frozen now, its digest and language the policy's, the policy cited, the
//! AS as its approver. The renewed grant has the policy's terms and digest,
//! so its own live-grant skip answers the next authorization until it
//! expires.

use pierre_auth::oauth2_server::authorization_details::{
    athlete_zone, preset_defaults, review_digest, Preset, ResolvedDetails, Tier,
};
use pierre_auth::oauth2_server::endpoints::{
    AdminAccess, CheckedAuthorization, OAuth2AuthorizationServer,
};
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::models::{GrantActor, GrantApproval, OAuthStandingPolicy, User};
use tracing::{error, info, warn};

use super::consent::ApprovedGrant;
use super::consent_text::ConsentText;
use super::consent_version::{consent_version, ConsentSubject};
use super::picker;
use super::{OAuth2Context, OAuth2Routes};

/// The highest tier a standing policy renews.
const MAX_RENEWED_TIER: Tier = Tier::T1;

/// The policy among `policies`, the athlete's live policies for the client
/// `subject` names, that renews `checked` without the screen, and the
/// details the renewed grant is approved for; `None` shows the screen. A
/// policy renews only the scope it was approved with: `scope` is what the
/// request would be granted.
pub(super) fn renewing_policy<'policy>(
    policies: &'policy [OAuthStandingPolicy],
    checked: &CheckedAuthorization,
    scope: &str,
    strings: &MessagingStringsRegistry,
    subject: ConsentSubject<'_>,
) -> Option<(&'policy OAuthStandingPolicy, ResolvedDetails)> {
    let (policy, requested) = if let Some(asked) = &checked.details {
        let preset = asked.preset().name();
        let policy = policies.iter().find(|policy| policy.preset == preset)?;
        (policy, asked.clone())
    } else {
        let source = checked
            .preset_source
            .as_deref()
            .filter(|_| picker::offered(strings))?;
        // Two live policies for one client: the AS never picks a preset.
        let [policy] = policies else {
            return None;
        };
        let preset = Preset::from_name(&policy.preset)?;
        (policy, preset_defaults(preset, source))
    };
    let terms = ResolvedDetails::from_document(&policy.authorization_details)
        .inspect_err(
            |e| error!(policy_id = %policy.id, "A standing policy's terms do not read: {e}"),
        )
        .ok()?;
    if policy.scope != scope
        || !requested.same_terms(&terms)
        || requested.max_tier() > MAX_RENEWED_TIER
    {
        return None;
    }
    let current = consent_version(&ConsentText::new(strings, &policy.consent_locale), subject);
    let digest = review_digest(&current, Some(&terms))
        .inspect_err(|e| error!(policy_id = %policy.id, "A standing policy's digest could not be computed: {e}"))
        .ok()?;
    (digest == policy.consent_version).then_some((policy, requested))
}

impl OAuth2Routes {
    /// Renew `checked` for `user` in `tenant_id` under one of their standing
    /// policies for the client `subject` names: the id of the grant recorded,
    /// or `None` when no policy renews it, or the renewal could not be
    /// recorded (logged), and the screen is shown.
    pub(super) async fn renew_under_standing_policy(
        context: &OAuth2Context,
        user: &User,
        tenant_id: &str,
        checked: &CheckedAuthorization,
        subject: ConsentSubject<'_>,
    ) -> Option<String> {
        let client_id = subject.client_id;
        let policies = context
            .standing_policies
            .list_standing_policies(tenant_id, &user.id.to_string(), Some(client_id))
            .await
            .inspect_err(|e| {
                error!(user_id = %user.id, client_id, "Failed to read standing policies, showing consent: {e}");
            })
            .ok()?;
        if policies.is_empty() {
            return None;
        }
        let scope = OAuth2AuthorizationServer::granted_scope(checked, AdminAccess::Withheld);
        let (policy, details) =
            renewing_policy(&policies, checked, &scope, &context.strings, subject)?;
        // A client first seen in this flow gets its row before the grant
        // naming it, as on the consent screen.
        if let Err(error) = Self::authorization_server(context)
            .enroll_checked_client(checked)
            .await
        {
            warn!(user_id = %user.id, client_id, "A standing policy's renewal could not enroll its client, showing consent: {error:?}");
            return None;
        }
        let renewed = Self::record_approval(
            context,
            ApprovedGrant {
                client_id,
                scope: &scope,
                details: Some(&details),
                consent_version: policy.consent_version.clone(),
                consent_locale: policy.consent_locale.clone(),
                zone: athlete_zone(user.timezone.as_deref()),
                approval: GrantApproval {
                    actor: GrantActor::Auto,
                    standing_policy: Some(policy.reference()),
                },
            },
            user.id,
            tenant_id,
        )
        .await
        .inspect_err(|e| {
            error!(user_id = %user.id, client_id, policy_id = %policy.id, "A standing policy's renewal could not be recorded, showing consent: {e}");
        })
        .ok()?;
        info!(user_id = %user.id, client_id, policy_id = %policy.id, client_grant_id = %renewed, "Preset grant renewed under a standing policy");
        Some(renewed)
    }
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_contremaitre::consent_strings::KEY_CONSENT_EXPRESS;
    use serde_json::Value;

    use super::*;

    const SOURCE: &str = "https://api.dravr.test/mcp";
    const CLIENT: &str = "https://client.example/meta.json";
    const SCOPE: &str = "fitness:read";

    fn subject() -> ConsentSubject<'static> {
        ConsentSubject {
            client_id: CLIENT,
            redirect_uri: "https://client.example/callback",
            first_party_only_sources: &[],
        }
    }

    /// A scope-only request the picker answers, or one naming `details`.
    fn checked(details: Option<ResolvedDetails>) -> CheckedAuthorization {
        CheckedAuthorization {
            scope: SCOPE.to_owned(),
            resource: None,
            client_name: None,
            consent_every_time: false,
            unstored_client: None,
            admin_grantable: false,
            details,
            preset_source: Some(SOURCE.to_owned()),
        }
    }

    /// A live policy for `terms`, ticked under the current English text.
    fn policy(strings: &MessagingStringsRegistry, terms: &ResolvedDetails) -> OAuthStandingPolicy {
        let version = consent_version(&ConsentText::new(strings, "en"), subject());
        OAuthStandingPolicy {
            id: format!("policy-{}", terms.preset().name()),
            tenant_id: "tenant".to_owned(),
            user_id: "user".to_owned(),
            client_id: CLIENT.to_owned(),
            preset: terms.preset().name().to_owned(),
            scope: SCOPE.to_owned(),
            authorization_details: terms.terms().to_document().unwrap(),
            consent_version: review_digest(&version, Some(terms)).unwrap(),
            consent_locale: "en".to_owned(),
            created_at: Timestamp::now(),
            revoked_at: None,
        }
    }

    #[test]
    fn a_scope_only_request_renews_the_one_policy_as_the_picker_resolves_it() {
        let strings = MessagingStringsRegistry::new();
        let terms = preset_defaults(Preset::TrainingCoach, SOURCE);
        let policies = [policy(&strings, &terms)];
        let (renewing, details) =
            renewing_policy(&policies, &checked(None), SCOPE, &strings, subject())
                .expect("bug: the policy's own preset is not renewed");
        assert_eq!(renewing.id, policies[0].id, "bug: another policy renewed");
        assert!(
            details.same_terms(&terms),
            "bug: the renewal is not for the policy's terms"
        );
    }

    #[test]
    fn a_policy_never_renews_another_scope() {
        let strings = MessagingStringsRegistry::new();
        let terms = preset_defaults(Preset::TrainingCoach, SOURCE);
        let policies = [policy(&strings, &terms)];
        for wider in [
            "fitness:read fitness:write",
            "fitness:read profile:read",
            "profile:read",
        ] {
            assert!(
                renewing_policy(&policies, &checked(None), wider, &strings, subject()).is_none(),
                "bug: a policy approved for `{SCOPE}` renewed `{wider}`"
            );
        }
    }

    #[test]
    fn two_policies_for_one_client_show_the_screen_to_a_scope_only_request() {
        let strings = MessagingStringsRegistry::new();
        let policies = [
            policy(&strings, &preset_defaults(Preset::TrainingCoach, SOURCE)),
            policy(&strings, &preset_defaults(Preset::SleepCoach, SOURCE)),
        ];
        assert!(
            renewing_policy(&policies, &checked(None), SCOPE, &strings, subject()).is_none(),
            "bug: the AS picked a preset between two policies"
        );
        let asked = preset_defaults(Preset::SleepCoach, SOURCE);
        let renewed = renewing_policy(&policies, &checked(Some(asked)), SCOPE, &strings, subject());
        assert_eq!(
            renewed.map(|(policy, _)| policy.preset.as_str()),
            Some("sleep_coach"),
            "bug: a request naming its preset is not renewed by that preset's policy"
        );
    }

    #[test]
    fn other_terms_a_changed_text_or_a_tier_past_t1_show_the_screen() {
        let strings = MessagingStringsRegistry::new();
        let terms = preset_defaults(Preset::TrainingCoach, SOURCE);
        let policies = [policy(&strings, &terms)];

        let mut narrowed: Value = serde_json::from_str(&terms.to_document().unwrap()).unwrap();
        narrowed[1]["budgets"]["requests_per_hour"] = Value::from(1);
        let narrowed = ResolvedDetails::from_document(&narrowed.to_string()).unwrap();
        assert!(
            renewing_policy(
                &policies,
                &checked(Some(narrowed)),
                SCOPE,
                &strings,
                subject()
            )
            .is_none(),
            "bug: a request with other terms was renewed"
        );

        let mut t2: Value = serde_json::from_str(&terms.to_document().unwrap()).unwrap();
        t2[1]["max_tier"] = Value::from("T2");
        let t2 = ResolvedDetails::from_document(&t2.to_string()).unwrap();
        let t2_policies = [policy(&strings, &t2)];
        assert!(
            renewing_policy(&t2_policies, &checked(Some(t2)), SCOPE, &strings, subject()).is_none(),
            "bug: a T2 grant was renewed without the screen"
        );

        strings.update(
            KEY_CONSENT_EXPRESS,
            "en",
            "Another wording.".to_owned(),
            "reworded".to_owned(),
        );
        assert!(
            renewing_policy(&policies, &checked(None), SCOPE, &strings, subject()).is_none(),
            "bug: a policy ticked under an older consent text renewed"
        );
    }
}
