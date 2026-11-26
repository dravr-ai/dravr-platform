// ABOUTME: The notices and consents a provider asks of an account: terms exposure, AI use, transfer, and the consent classes
// ABOUTME: One table (PROVIDER_NOTICES) with whether a connect requires or only offers each; the keys consents are recorded under
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider notices and consents ([`PROVIDER_NOTICES`]): what each asks of
//! an account, whether a connect waits for it, and the key its answer is
//! recorded under.

use super::{GARMIN, SCIOTTE_COROS, SCIOTTE_GARMIN, SCIOTTE_TRAININGPEAKS, WHOOP};

/// Which accounts a provider's notice is asked of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAudience {
    /// Every account that connects the provider, whatever its feature flags:
    /// the provider's own terms bind every connection, so no admin switch
    /// can waive the notice.
    EveryAccount,
    /// Only the accounts the `provider_exposure_notice` feature flag arms,
    /// per tenant or per user; an account the flag leaves off is never asked.
    FlagArmedAccounts,
}

/// Whether a connect waits for a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtConnect {
    /// The connect is refused until the account accepts the notice.
    Required,
    /// The connect offers the notice as a box the athlete may leave unticked:
    /// the connect goes through either way, and the athlete gives or
    /// withdraws it later in Privacy & Data (JF 2026-11-23, carnet#1157).
    Offered,
}

/// A notice a provider asks the account to accept, by the key its acceptance
/// is recorded under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderNotice {
    /// The key the acceptance is recorded under: the backend the notice
    /// guards, a consent class, or a consent of its own
    /// ([`GARMIN_WORKOUT_TRANSFER`]).
    pub backend: &'static str,
    /// The backends the notice is about: a connect to one offers or requires
    /// it ([`AtConnect`]), and its consent stands for every one of them. A
    /// consent class covers none.
    pub covers: &'static [&'static str],
    /// Whether a connect to a covered backend waits for it.
    pub at_connect: AtConnect,
    /// The notice's dated version; an acceptance of any other version is no
    /// acceptance of this one.
    pub version: &'static str,
    /// Which accounts are asked for it.
    pub audience: NoticeAudience,
    /// What accepting it gives Dravr, and so whether it can be withdrawn.
    pub kind: NoticeKind,
}

/// What accepting a provider's notice gives Dravr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// An acknowledgement of a risk the provider's terms of use create. It
    /// gates the connect and nothing after it, so there is nothing to
    /// withdraw: a disconnect ends the exposure.
    TermsExposure,
    /// Consent to hand the provider's data — or, for a consent class
    /// ([`CONSENT_CLASSES`]), every record of its tier — to an AI model,
    /// which the athlete can withdraw at any time, as simply as it was given.
    /// While it is not given, or once it is withdrawn, no model reads that
    /// data; the athlete still sees what is stored and the connection stays
    /// live.
    AiConsent,
    /// Consent to send the athlete's data to the provider, which the provider's
    /// terms require before the first transfer (Garmin Connect Developer
    /// Program Agreement §4.5, carnet#1390) and which the athlete can withdraw
    /// at any time. It withholds no read: while it is not given, Dravr writes
    /// nothing to the provider.
    TransferConsent,
}

impl ProviderNotice {
    /// Whether the account can withdraw its acceptance once given: a consent
    /// to AI use or to a transfer can be; a terms-of-use exposure cannot.
    #[must_use]
    pub const fn withdrawable(&self) -> bool {
        matches!(
            self.kind,
            NoticeKind::AiConsent | NoticeKind::TransferConsent
        )
    }

    /// Whether this notice covers `backend` ([`Self::covers`]).
    #[must_use]
    pub fn covers(&self, backend: &str) -> bool {
        self.covers.contains(&backend)
    }
}

/// Every notice a provider requires the account to accept before connecting,
/// by the backend it guards.
///
/// Two kinds of notice share this table and the one acceptance record behind
/// it:
///
/// - **Terms-of-use exposure.** A provider read by signing in with the
///   athlete's own account, against its terms of use, states the risk before
///   the credentials: TrainingPeaks (Terms of Use section 13) and COROS (Terms
///   of Service sections 4 and 7). Asked of the accounts the
///   `provider_exposure_notice` flag arms
///   ([`NoticeAudience::FlagArmedAccounts`]). A [`NoticeKind::TermsExposure`]:
///   nothing to withdraw.
/// - **Owner authorization.** WHOOP's API Terms of Use (section 4, effective
///   2026-10-06) let Dravr store WHOOP Data, compute from it and hand it to
///   the coach only as the data's owner expressly authorizes; the athlete
///   gives that authorization before the WHOOP OAuth flow begins, and health
///   sync keeps no WHOOP record for an account that has not. The terms bind
///   every WHOOP connection, so it is asked of every account
///   ([`NoticeAudience::EveryAccount`]). A [`NoticeKind::AiConsent`]: the
///   athlete withdraws it from the privacy settings or the connection card,
///   after which no model reads WHOOP data and health sync keeps no new WHOOP
///   record until it is given again (carnet#726).
///
/// - **Consent classes.** [`SENSITIVE_T2`] and [`SENSITIVE_T3`] are no
///   provider: each is the athlete's consent to AI use of every record of
///   one sensitivity tier, owl's T2 (labs, clinical records, self-measured
///   glucose) and T3 (cycle tracking, mental and reproductive health),
///   whichever provider or import holds it (J2, 2026-11-12). Neither opens
///   T3 to an outside AI client: T3 never goes outside (JF
///   2026-11-25, closing JF 2026-11-20's path, carnet#1544), and Dravr's own
///   agent and the human coach read the athlete's notes and their agent's
///   memory either way. A class guards no connect, since nothing
///   connects under it; it rides this table only so the one AI-consent set
///   withholds it until given, exactly like WHOOP's authorization, and so one
///   acceptance record holds every consent to AI use. Asked of every account
///   ([`NoticeAudience::EveryAccount`]) as a [`NoticeKind::AiConsent`]: given
///   and withdrawn from the privacy settings. No provider descriptor is
///   registered under its key ([`is_consent_class`]); a refusal or a route
///   names it by [`sensitive_display_name`], never by the key.
///
/// - **Garmin.** Garmin's agreement (v9 §15.10) lets its data reach an AI
///   model only with the athlete's explicit, withdrawable consent, and §4.5
///   requires a notice and the athlete's express consent before any of their
///   data is sent to Garmin. [`GARMIN`] is the AI consent, a
///   [`NoticeKind::AiConsent`] covering the official API and the sign-in
///   capture alike (JF 2026-11-23): Garmin items carry `garmin` as their
///   provider or source either way, and a read checked by backend name maps
///   the capture to the same key ([`consent_key`]). [`GARMIN_WORKOUT_TRANSFER`]
///   is the transfer consent, a [`NoticeKind::TransferConsent`] gating every
///   workout Dravr writes to the Garmin Connect calendar (carnet#1390). Both
///   are [`AtConnect::Offered`]: the connect shows them as two unticked boxes
///   and goes through whatever the athlete ticks.
///
/// A connect to a backend an [`AtConnect::Required`] notice covers — a
/// credential login or the start of an OAuth flow — is refused until an
/// account asked for the notice accepts its current version; the web and
/// mobile connect surfaces and the hosted pages show it. An account that
/// accepted a version connects again without being asked. Bump a provider's
/// version whenever its notice text changes on any
/// surface or in any locale, so every account is asked again.
pub const PROVIDER_NOTICES: [ProviderNotice; 7] = [
    ProviderNotice {
        backend: SCIOTTE_TRAININGPEAKS,
        covers: &[SCIOTTE_TRAININGPEAKS],
        at_connect: AtConnect::Required,
        version: "2026-09-24",
        audience: NoticeAudience::FlagArmedAccounts,
        kind: NoticeKind::TermsExposure,
    },
    ProviderNotice {
        backend: SCIOTTE_COROS,
        covers: &[SCIOTTE_COROS],
        at_connect: AtConnect::Required,
        version: "2026-09-24",
        audience: NoticeAudience::FlagArmedAccounts,
        kind: NoticeKind::TermsExposure,
    },
    ProviderNotice {
        backend: WHOOP,
        covers: &[WHOOP],
        at_connect: AtConnect::Required,
        version: "2026-09-25",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::AiConsent,
    },
    ProviderNotice {
        backend: SENSITIVE_T2,
        covers: &[],
        at_connect: AtConnect::Required,
        version: "2026-11-20",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::AiConsent,
    },
    ProviderNotice {
        backend: SENSITIVE_T3,
        covers: &[],
        at_connect: AtConnect::Required,
        version: "2026-11-20",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::AiConsent,
    },
    ProviderNotice {
        backend: GARMIN,
        covers: &[GARMIN, SCIOTTE_GARMIN],
        at_connect: AtConnect::Offered,
        version: "2026-11-23",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::AiConsent,
    },
    ProviderNotice {
        backend: GARMIN_WORKOUT_TRANSFER,
        covers: &[GARMIN, SCIOTTE_GARMIN],
        at_connect: AtConnect::Offered,
        version: "2026-11-23",
        audience: NoticeAudience::EveryAccount,
        kind: NoticeKind::TransferConsent,
    },
];

/// The key of the athlete's consent to Dravr sending their workouts to the
/// Garmin Connect calendar through Garmin's Training API (carnet#1390).
///
/// No provider is registered under it: it is a consent of its own, recorded
/// as a row of `provider_terms_consents`, offered at the Garmin connect and
/// given or withdrawn in Privacy & Data ([`PROVIDER_NOTICES`]).
pub const GARMIN_WORKOUT_TRANSFER: &str = "garmin_workout_transfer";

/// Garmin's privacy policy, which the transfer notice links to (Garmin's
/// agreement §4.5 requires the link beside the notice).
pub const GARMIN_PRIVACY_URL: &str = "https://www.garmin.com/privacy/connect";

/// The key the `kind` consent covering `backend` is recorded under, or `None`
/// when no such consent covers it.
///
/// A backend checked by name — the sign-in capture's `sciotte_garmin` — and
/// its official API share one consent (JF 2026-11-23); a consent class names
/// itself.
#[must_use]
pub fn consent_key(backend: &str, kind: NoticeKind) -> Option<&'static str> {
    PROVIDER_NOTICES
        .iter()
        .find(|notice| notice.kind == kind && (notice.backend == backend || notice.covers(backend)))
        .map(|notice| notice.backend)
}

/// The notices a connect to `backend` offers as boxes the athlete may leave
/// unticked ([`AtConnect::Offered`]), by key.
pub fn offered_at_connect(backend: &str) -> impl Iterator<Item = &'static str> + '_ {
    PROVIDER_NOTICES
        .iter()
        .filter(move |notice| notice.at_connect == AtConnect::Offered && notice.covers(backend))
        .map(|notice| notice.backend)
}

/// The consent class of the athlete's T2 records.
///
/// Labs, medications, conditions, clinical records and self-measured
/// glucose, whichever provider or import holds them: while it is not given,
/// no model and no human coach reads one ([`PROVIDER_NOTICES`]). Its
/// acceptance is a row of `provider_terms_consents` under this key.
pub const SENSITIVE_T2: &str = "sensitive_t2";

/// The consent class of the athlete's T3 records.
///
/// Cycle tracking, mental, reproductive and substance records, whichever
/// provider or import holds them: while it is not given, no model and no
/// human coach reads one. No outside AI client (MCP, A2A, an API key) reads
/// a T3 record or the athlete's notes and their agent's memory, given or not
/// (JF 2026-11-25); Dravr's own agent and the human coach read the
/// notes and memory without it ([`PROVIDER_NOTICES`]). Its acceptance is a row of
/// `provider_terms_consents` under this key.
pub const SENSITIVE_T3: &str = "sensitive_t3";

/// Every consent class [`PROVIDER_NOTICES`] carries.
pub const CONSENT_CLASSES: [&str; 2] = [SENSITIVE_T2, SENSITIVE_T3];

/// Whether `key` names a consent class rather than a provider.
#[must_use]
pub fn is_consent_class(key: &str) -> bool {
    CONSENT_CLASSES.contains(&key)
}

/// The name a refusal and a route give a consent class, in place of its key;
/// `None` for any other key.
#[must_use]
pub fn sensitive_display_name(key: &str) -> Option<&'static str> {
    match key {
        SENSITIVE_T2 => Some("Health records"),
        SENSITIVE_T3 => Some("Intimate health records"),
        _ => None,
    }
}

/// The backends whose notice is a consent to AI use
/// ([`NoticeKind::AiConsent`]): while an account has not given it, or once it
/// withdrew it, no model reads that backend's data.
pub fn ai_consent_backends() -> impl Iterator<Item = &'static str> {
    PROVIDER_NOTICES
        .iter()
        .filter(|notice| notice.kind == NoticeKind::AiConsent)
        .map(|notice| notice.backend)
}

/// The notice `backend` requires, or `None` when the provider asks for none.
#[must_use]
pub fn provider_notice(backend: &str) -> Option<&'static ProviderNotice> {
    PROVIDER_NOTICES
        .iter()
        .find(|notice| notice.backend == backend)
}

/// The current notice version for `backend`, or `None` when the provider
/// asks for no notice.
#[must_use]
pub fn provider_terms_version(backend: &str) -> Option<&'static str> {
    provider_notice(backend).map(|notice| notice.version)
}

#[cfg(test)]
mod tests {
    use super::{
        ai_consent_backends, consent_key, offered_at_connect, provider_notice, AtConnect,
        NoticeKind, ProviderNotice, GARMIN, GARMIN_WORKOUT_TRANSFER, SCIOTTE_COROS, SCIOTTE_GARMIN,
        SCIOTTE_TRAININGPEAKS, SENSITIVE_T2, WHOOP,
    };

    #[test]
    fn one_garmin_ai_consent_covers_the_api_and_the_sign_in() {
        for backend in [GARMIN, SCIOTTE_GARMIN] {
            assert_eq!(
                consent_key(backend, NoticeKind::AiConsent),
                Some(GARMIN),
                "bug: {backend} answers to another AI consent than Garmin's"
            );
            assert_eq!(
                consent_key(backend, NoticeKind::TransferConsent),
                Some(GARMIN_WORKOUT_TRANSFER),
                "bug: {backend} answers to another transfer consent than Garmin's"
            );
        }
        assert!(
            ai_consent_backends().any(|key| key == GARMIN),
            "bug: Garmin's AI consent withholds no read"
        );
        assert!(
            !ai_consent_backends().any(|key| key == GARMIN_WORKOUT_TRANSFER),
            "a transfer consent withholds no read"
        );
    }

    #[test]
    fn a_garmin_connect_offers_both_boxes_and_requires_neither() {
        for backend in [GARMIN, SCIOTTE_GARMIN] {
            assert_eq!(
                offered_at_connect(backend).collect::<Vec<_>>(),
                vec![GARMIN, GARMIN_WORKOUT_TRANSFER],
                "bug: a connect to {backend} offers other boxes than Garmin's two"
            );
        }
        for key in [GARMIN, GARMIN_WORKOUT_TRANSFER] {
            let notice = provider_notice(key);
            assert_eq!(
                notice.map(|notice| notice.at_connect),
                Some(AtConnect::Offered),
                "bug: {key} is not a box the connect offers"
            );
            assert!(
                notice.is_some_and(ProviderNotice::withdrawable),
                "bug: {key} cannot be withdrawn"
            );
        }
    }

    #[test]
    fn the_notices_a_connect_waits_for_still_gate_it() {
        for key in [WHOOP, SCIOTTE_TRAININGPEAKS, SCIOTTE_COROS] {
            assert_eq!(
                provider_notice(key).map(|notice| notice.at_connect),
                Some(AtConnect::Required),
                "bug: a connect to {key} no longer waits for its notice"
            );
            assert_eq!(offered_at_connect(key).count(), 0, "{key} offers no box");
        }
        assert_eq!(
            consent_key(SENSITIVE_T2, NoticeKind::AiConsent),
            Some(SENSITIVE_T2),
            "a consent class names itself"
        );
        assert_eq!(
            consent_key("strava", NoticeKind::AiConsent),
            None,
            "bug: Strava asks for an AI consent"
        );
    }
}
