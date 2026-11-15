// ABOUTME: Catalogue keys of the OAuth consent screen and the refusal pages of its flow
// ABOUTME: One constant per string, each with a positional slot wherever the page fills in a value
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the OAuth consent flow's server-rendered pages.
//!
//! `pierre-routes-identity` renders them in the reader's language. The
//! outside-Québec and express-consent lines carry legal weight, so a change to
//! their wording in any locale is a change a person reads, not a copy edit.

/// Key: "Authorize access": the page title and heading. No format placeholders.
pub const KEY_CONSENT_TITLE: &str = "oauth.consent.title";
/// Key: "`{0}` wants to access your Dravr fitness data."
pub const KEY_CONSENT_LEAD: &str = "oauth.consent.lead";
/// Key: "It will return you to `{0}`"
pub const KEY_CONSENT_RETURNS_TO: &str = "oauth.consent.returnsTo";
/// Key: "This will allow it to", above the scopes. No format placeholders.
pub const KEY_CONSENT_SCOPES_TITLE: &str = "oauth.consent.scopesTitle";
/// Key: what `fitness:read` allows. No format placeholders.
pub const KEY_CONSENT_SCOPE_FITNESS_READ: &str = "oauth.consent.scope.fitnessRead";
/// Key: what `fitness:write` allows. No format placeholders.
pub const KEY_CONSENT_SCOPE_FITNESS_WRITE: &str = "oauth.consent.scope.fitnessWrite";
/// Key: what `profile:read` allows. No format placeholders.
pub const KEY_CONSENT_SCOPE_PROFILE_READ: &str = "oauth.consent.scope.profileRead";
/// Key: what `profile:write` allows. No format placeholders.
pub const KEY_CONSENT_SCOPE_PROFILE_WRITE: &str = "oauth.consent.scope.profileWrite";
/// Key: what `offline_access` allows. No format placeholders.
pub const KEY_CONSENT_SCOPE_OFFLINE_ACCESS: &str = "oauth.consent.scope.offlineAccess";
/// Key: "Allow access", the approving button. No format placeholders.
pub const KEY_CONSENT_ALLOW: &str = "oauth.consent.allow";
/// Key: "Deny", the button that refuses. No format placeholders.
pub const KEY_CONSENT_DENY: &str = "oauth.consent.deny";
/// Key: "Signed in as `{0}`"
pub const KEY_CONSENT_SIGNED_IN_AS: &str = "oauth.consent.signedInAs";
/// Key: "Use another account". No format placeholders.
pub const KEY_CONSENT_USE_ANOTHER_ACCOUNT: &str = "oauth.consent.useAnotherAccount";
/// Key: that the access can be revoked in settings. No format placeholders.
pub const KEY_CONSENT_REVOKE_ANYTIME: &str = "oauth.consent.revokeAnytime";
/// Key: "Privacy policy". No format placeholders.
pub const KEY_CONSENT_PRIVACY_POLICY: &str = "oauth.consent.privacyPolicy";
/// Key: "Terms". No format placeholders.
pub const KEY_CONSENT_TERMS: &str = "oauth.consent.terms";
/// Key: "Verified: `{0}` by `{1}`", the product and its operator
pub const KEY_CONSENT_VERIFIED: &str = "oauth.consent.verified";
/// Key: "Unverified app." No format placeholders.
pub const KEY_CONSENT_UNVERIFIED_TITLE: &str = "oauth.consent.unverifiedTitle";
/// Key: that nobody verified who runs the app. No format placeholders.
pub const KEY_CONSENT_UNVERIFIED_BODY: &str = "oauth.consent.unverifiedBody";
/// Key: the host publishing an unverified app's metadata document, `{0}`
pub const KEY_CONSENT_UNVERIFIED_PUBLISHED_BY: &str = "oauth.consent.unverifiedPublishedBy";
/// Key: that the app returns to a program on this device. No format placeholders.
pub const KEY_CONSENT_UNVERIFIED_LOOPBACK: &str = "oauth.consent.unverifiedLoopback";
/// Key: what data reaches the client `{0}`, and that it is not medical advice
pub const KEY_CONSENT_HEALTH: &str = "oauth.consent.health";
/// Key: Law 25 s.8 for a known client: `{0}` is run by `{1}` and processes `{2}`
pub const KEY_CONSENT_OUTSIDE_QUEBEC_KNOWN: &str = "oauth.consent.outsideQuebecKnown";
/// Key: Law 25 s.8 for any other client `{0}`
pub const KEY_CONSENT_OUTSIDE_QUEBEC_UNKNOWN: &str = "oauth.consent.outsideQuebecUnknown";
/// Key: "in the United States", preposition included, which `{2}` above reads
pub const KEY_CONSENT_IN_UNITED_STATES: &str = "oauth.consent.inUnitedStates";
/// Key: Law 25 s.12: choosing the button `{0}` expressly consents to sharing with `{1}`
pub const KEY_CONSENT_EXPRESS: &str = "oauth.consent.express";
/// Key: data from the services `{0}` stays inside Dravr and never reaches `{1}`
pub const KEY_CONSENT_FIRST_PARTY_ONLY: &str = "oauth.consent.firstPartyOnly";
/// Key: "`{0}` and `{1}`", joining the last service to the ones before it
pub const KEY_CONSENT_LIST_PAIR: &str = "oauth.consent.listPair";

/// Key: "Connection Failed": the error page's title and heading. No format placeholders.
pub const KEY_ERROR_TITLE: &str = "oauth.error.title";
/// Key: "Please try again." No format placeholders.
pub const KEY_ERROR_TRY_AGAIN: &str = "oauth.error.tryAgain";
/// Key: a stale or forged consent form. No format placeholders.
pub const KEY_ERROR_CONSENT_EXPIRED: &str = "oauth.error.consentExpired";
/// Key: the form token could not be minted. No format placeholders.
pub const KEY_ERROR_CONSENT_UNAVAILABLE: &str = "oauth.error.consentUnavailable";
/// Key: an account waiting on its confirmation link. No format placeholders.
pub const KEY_ERROR_AWAITS_EMAIL_PROOF: &str = "oauth.error.awaitsEmailProof";
/// Key: an account waiting on an operator. No format placeholders.
pub const KEY_ERROR_AWAITS_APPROVAL: &str = "oauth.error.awaitsApproval";
/// Key: a suspended account. No format placeholders.
pub const KEY_ERROR_SUSPENDED: &str = "oauth.error.suspended";
/// Key: a session naming an account that is gone. No format placeholders.
pub const KEY_ERROR_ACCOUNT_GONE: &str = "oauth.error.accountGone";
/// Key: the account could not be read. No format placeholders.
pub const KEY_ERROR_ACCOUNT_UNAVAILABLE: &str = "oauth.error.accountUnavailable";
/// Key: past the window's allowance. No format placeholders.
pub const KEY_ERROR_RATE_LIMITED: &str = "oauth.error.rateLimited";
/// Key: the limiter could not count the request. No format placeholders.
pub const KEY_ERROR_LIMITER_UNAVAILABLE: &str = "oauth.error.limiterUnavailable";
/// Key: Google or the account rules turned the sign-in down. No format placeholders.
pub const KEY_ERROR_GOOGLE_REFUSED: &str = "oauth.error.googleRefused";
/// Key: a Google callback this browser did not start. No format placeholders.
pub const KEY_ERROR_GOOGLE_EXPIRED: &str = "oauth.error.googleExpired";
/// Key: the operator's sign-up bounds admit no new account. No format placeholders.
pub const KEY_ERROR_SIGNUP_PAUSED: &str = "oauth.error.signupPaused";
/// Key: any other `invalid_request`. No format placeholders.
pub const KEY_ERROR_INVALID_REQUEST: &str = "oauth.error.invalidRequest";
/// Key: any other `invalid_client`. No format placeholders.
pub const KEY_ERROR_INVALID_CLIENT: &str = "oauth.error.invalidClient";
/// Key: any other `access_denied`. No format placeholders.
pub const KEY_ERROR_DENIED: &str = "oauth.error.denied";
/// Key: any other `server_error`. No format placeholders.
pub const KEY_ERROR_SERVER: &str = "oauth.error.serverError";
/// Key: any other `temporarily_unavailable`. No format placeholders.
pub const KEY_ERROR_UNAVAILABLE: &str = "oauth.error.unavailable";
