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
/// Key: [`KEY_CONSENT_SCOPE_FITNESS_READ`] on a preset grant's screen, without
/// the injuries and activity places a preset never shares. No format placeholders.
pub const KEY_CONSENT_SCOPE_FITNESS_READ_PRESET: &str = "oauth.consent.scope.fitnessReadPreset";
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
/// Key: "Verified: `{0}` by `{1}`", the product and the company behind it
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
/// Key: [`KEY_CONSENT_HEALTH`] on a preset grant's screen: the data listed
/// below reaches the client `{0}`, and it is not medical advice
pub const KEY_CONSENT_HEALTH_PRESET: &str = "oauth.consent.healthPreset";
/// Key: Law 25 s.8 for a known client: `{0}` is run by `{1}` and processes `{2}`
pub const KEY_CONSENT_OUTSIDE_QUEBEC_KNOWN: &str = "oauth.consent.outsideQuebecKnown";
/// Key: Law 25 s.8 for any other client `{0}`
pub const KEY_CONSENT_OUTSIDE_QUEBEC_UNKNOWN: &str = "oauth.consent.outsideQuebecUnknown";
/// Key: "in the United States", preposition included, which `{2}` above reads
pub const KEY_CONSENT_IN_UNITED_STATES: &str = "oauth.consent.inUnitedStates";
/// Key: Law 25 s.12: choosing the button `{0}` expressly consents to sharing with `{1}`
pub const KEY_CONSENT_EXPRESS: &str = "oauth.consent.express";
/// Key: [`KEY_CONSENT_EXPRESS`] on a preset grant's screen: choosing the
/// button `{0}` expressly consents to sharing the listed data with `{1}`
pub const KEY_CONSENT_EXPRESS_PRESET: &str = "oauth.consent.expressPreset";
/// Key: data from the services `{0}` stays inside Dravr and never reaches `{1}`
pub const KEY_CONSENT_FIRST_PARTY_ONLY: &str = "oauth.consent.firstPartyOnly";
/// Key: "`{0}` and `{1}`", joining the last service to the ones before it
pub const KEY_CONSENT_LIST_PAIR: &str = "oauth.consent.listPair";
/// Key: shown to a human coach who leads an active group: `{0}`, the client,
/// also sees by name the training of the members who allowed their coach's AI
/// tools (carnet#1511)
pub const KEY_CONSENT_ROLE_COACH: &str = "oauth.consent.roleCoach";
/// Key: shown to an admin: `{0}`, the client, is offered every tool, and may
/// act as a user in a support session (carnet#1511)
pub const KEY_CONSENT_ROLE_ADMIN: &str = "oauth.consent.roleAdmin";
/// Key: the Admin access box's label, shown to an admin only (carnet#1511)
pub const KEY_CONSENT_ADMIN_ACCESS_LABEL: &str = "oauth.consent.adminAccessLabel";
/// Key: who sees the Admin access box, that it is asked every time, and where
/// to withdraw it
pub const KEY_CONSENT_ADMIN_ACCESS_HINT: &str = "oauth.consent.adminAccessHint";
/// Key: where admin access goes: `{0}`, the redirect host in bold, or
/// [`KEY_CONSENT_ADMIN_ACCESS_THIS_COMPUTER`]
pub const KEY_CONSENT_ADMIN_ACCESS_GOES_TO: &str = "oauth.consent.adminAccessGoesTo";
/// Key: where admin access goes when the client's metadata document is on
/// another host: `{0}` as above, `{1}` that host in bold
pub const KEY_CONSENT_ADMIN_ACCESS_GOES_TO_TWO_HOSTS: &str =
    "oauth.consent.adminAccessGoesToTwoHosts";
/// Key: what `{0}` reads for a loopback redirect: a program on this device
pub const KEY_CONSENT_ADMIN_ACCESS_THIS_COMPUTER: &str = "oauth.consent.adminAccessThisComputer";

/// Key: above the picker: choose what `{0}`, the client, can read
pub const KEY_CONSENT_GRANT_CHOICE_TITLE: &str = "oauth.consent.grantChoiceTitle";
/// Key: the picker's first choice, preselected: every tool, as today
pub const KEY_CONSENT_GRANT_EVERYTHING: &str = "oauth.consent.grantEverything";
/// Key: what Everything grants, and that it lasts until removed
pub const KEY_CONSENT_GRANT_EVERYTHING_HINT: &str = "oauth.consent.grantEverythingHint";
/// Key: the `training_coach` preset's name
pub const KEY_CONSENT_GRANT_TRAINING_COACH: &str = "oauth.consent.grantTrainingCoach";
/// Key: what `training_coach` reads: `{0}`, the days of data back
pub const KEY_CONSENT_GRANT_TRAINING_COACH_HINT: &str = "oauth.consent.grantTrainingCoachHint";
/// Key: what `training_coach` reads without raw samples: `{0}`, the days
/// of data back
pub const KEY_CONSENT_GRANT_TRAINING_COACH_SUMMARY_HINT: &str =
    "oauth.consent.grantTrainingCoachSummaryHint";
/// Key: the `sleep_coach` preset's name
pub const KEY_CONSENT_GRANT_SLEEP_COACH: &str = "oauth.consent.grantSleepCoach";
/// Key: what `sleep_coach` reads: `{0}`, the days of data back
pub const KEY_CONSENT_GRANT_SLEEP_COACH_HINT: &str = "oauth.consent.grantSleepCoachHint";
/// Key: how long a preset grant lasts: `{0}`, whole days
pub const KEY_CONSENT_GRANT_LASTS_DAYS: &str = "oauth.consent.grantLastsDays";
/// Key: how long a preset grant lasts: `{0}`, hours, under a day
pub const KEY_CONSENT_GRANT_LASTS_HOURS: &str = "oauth.consent.grantLastsHours";
/// Key: above a request's resolved details: `{0}`, the client, asks only for
pub const KEY_CONSENT_GRANT_REQUESTED: &str = "oauth.consent.grantRequested";
/// Key: the client's purpose, quoted: `{0}` the client, `{1}` its words
pub const KEY_CONSENT_GRANT_PURPOSE: &str = "oauth.consent.grantPurpose";
/// Key: above the client's commitments, each quoted: `{0}`, the client
pub const KEY_CONSENT_GRANT_COMMITMENTS: &str = "oauth.consent.grantCommitments";

/// Every line the consent screen can show.
///
/// What the user reads when they choose Allow access.
/// `pierre-routes-identity` versions the consent it records by this text, so
/// a key the screen renders and this list leaves out is a wording change
/// nobody is asked again about; its test holds every `oauth.consent.` key in
/// the catalogue to this list.
pub const CONSENT_SCREEN_KEYS: [&str; 51] = [
    KEY_CONSENT_TITLE,
    KEY_CONSENT_LEAD,
    KEY_CONSENT_RETURNS_TO,
    KEY_CONSENT_SCOPES_TITLE,
    KEY_CONSENT_SCOPE_FITNESS_READ,
    KEY_CONSENT_SCOPE_FITNESS_READ_PRESET,
    KEY_CONSENT_SCOPE_FITNESS_WRITE,
    KEY_CONSENT_SCOPE_PROFILE_READ,
    KEY_CONSENT_SCOPE_PROFILE_WRITE,
    KEY_CONSENT_SCOPE_OFFLINE_ACCESS,
    KEY_CONSENT_ALLOW,
    KEY_CONSENT_DENY,
    KEY_CONSENT_SIGNED_IN_AS,
    KEY_CONSENT_USE_ANOTHER_ACCOUNT,
    KEY_CONSENT_REVOKE_ANYTIME,
    KEY_CONSENT_PRIVACY_POLICY,
    KEY_CONSENT_TERMS,
    KEY_CONSENT_VERIFIED,
    KEY_CONSENT_UNVERIFIED_TITLE,
    KEY_CONSENT_UNVERIFIED_BODY,
    KEY_CONSENT_UNVERIFIED_PUBLISHED_BY,
    KEY_CONSENT_UNVERIFIED_LOOPBACK,
    KEY_CONSENT_HEALTH,
    KEY_CONSENT_HEALTH_PRESET,
    KEY_CONSENT_OUTSIDE_QUEBEC_KNOWN,
    KEY_CONSENT_OUTSIDE_QUEBEC_UNKNOWN,
    KEY_CONSENT_IN_UNITED_STATES,
    KEY_CONSENT_EXPRESS,
    KEY_CONSENT_EXPRESS_PRESET,
    KEY_CONSENT_FIRST_PARTY_ONLY,
    KEY_CONSENT_LIST_PAIR,
    KEY_CONSENT_ROLE_COACH,
    KEY_CONSENT_ROLE_ADMIN,
    KEY_CONSENT_ADMIN_ACCESS_LABEL,
    KEY_CONSENT_ADMIN_ACCESS_HINT,
    KEY_CONSENT_ADMIN_ACCESS_GOES_TO,
    KEY_CONSENT_ADMIN_ACCESS_GOES_TO_TWO_HOSTS,
    KEY_CONSENT_ADMIN_ACCESS_THIS_COMPUTER,
    KEY_CONSENT_GRANT_CHOICE_TITLE,
    KEY_CONSENT_GRANT_EVERYTHING,
    KEY_CONSENT_GRANT_EVERYTHING_HINT,
    KEY_CONSENT_GRANT_TRAINING_COACH,
    KEY_CONSENT_GRANT_TRAINING_COACH_HINT,
    KEY_CONSENT_GRANT_TRAINING_COACH_SUMMARY_HINT,
    KEY_CONSENT_GRANT_SLEEP_COACH,
    KEY_CONSENT_GRANT_SLEEP_COACH_HINT,
    KEY_CONSENT_GRANT_LASTS_DAYS,
    KEY_CONSENT_GRANT_LASTS_HOURS,
    KEY_CONSENT_GRANT_REQUESTED,
    KEY_CONSENT_GRANT_PURPOSE,
    KEY_CONSENT_GRANT_COMMITMENTS,
];

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
/// Key: an account waiting on an admin. No format placeholders.
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
/// Key: the admin's sign-up bounds admit no new account. No format placeholders.
pub const KEY_ERROR_SIGNUP_PAUSED: &str = "oauth.error.signupPaused";
/// Key: any other `invalid_request`. No format placeholders.
pub const KEY_ERROR_INVALID_REQUEST: &str = "oauth.error.invalidRequest";
/// Key: a pushed request's `request_uri` that expired or was already used.
/// No format placeholders.
pub const KEY_ERROR_INVALID_REQUEST_URI: &str = "oauth.error.invalidRequestUri";
/// Key: any other `invalid_client`. No format placeholders.
pub const KEY_ERROR_INVALID_CLIENT: &str = "oauth.error.invalidClient";
/// Key: any other `access_denied`. No format placeholders.
pub const KEY_ERROR_DENIED: &str = "oauth.error.denied";
/// Key: any other `server_error`. No format placeholders.
pub const KEY_ERROR_SERVER: &str = "oauth.error.serverError";
/// Key: any other `temporarily_unavailable`. No format placeholders.
pub const KEY_ERROR_UNAVAILABLE: &str = "oauth.error.unavailable";
