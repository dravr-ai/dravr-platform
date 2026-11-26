// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The provider capability-scope vocabulary — one word per wire slug, in the athlete's language
// ABOUTME: Also holds each sign-in target's login facts, each provider's notice to accept first, a card's history words, and how Intervals.icu links

import type { ExtendedProviderStatus, HistoryState, SciotteTarget } from '@pierre/shared-types';

/**
 * Every capability slug `GET /api/oauth/providers` can put on a provider card,
 * in the order the route builds them (`crates/pierre-routes-auth/src/oauth.rs`).
 * The strings are the wire values, so they stay English whatever the athlete
 * reads.
 */
export const PROVIDER_SCOPES = [
  'activities',
  'sleep',
  'recovery',
  'health',
  'planned_workouts',
] as const;

export type ProviderScope = (typeof PROVIDER_SCOPES)[number];

/**
 * The corpus key naming each scope. Module scope cannot hold a hook, so the
 * table carries the key and each client resolves it with its own `t` — the
 * card's capability line then reads as words in the athlete's language
 * instead of as the wire slugs under otherwise translated chrome.
 */
export const PROVIDER_SCOPE_LABEL_KEY: Record<ProviderScope, string> = {
  activities: 'providers.scope.activities',
  sleep: 'providers.scope.sleep',
  recovery: 'providers.scope.recovery',
  health: 'providers.scope.health',
  planned_workouts: 'providers.scope.planned_workouts',
};

/**
 * The label key for `scope`, or `null` for a slug the catalogue has no word
 * for. A caller prints such a slug as itself: a provider that starts
 * advertising a new capability shows its wire name, never a missing-key
 * string.
 */
export function providerScopeLabelKey(scope: string): string | null {
  return PROVIDER_SCOPE_LABEL_KEY[scope as ProviderScope] ?? null;
}

/** The catalogue keys of the notice a provider asks for before connecting. */
export interface ProviderNoticeKeys {
  titleKey: string;
  bodyKey: string;
  consentKey: string;
}

/**
 * Every provider notice, by the backend id the server's `consent_required`
 * names it by (`PROVIDER_NOTICES` in
 * `crates/pierre-core/src/constants/oauth/providers.rs`, which also says which
 * accounts the server asks for each). TrainingPeaks and COROS state the
 * account risk of a session read; WHOOP asks every account for the owner
 * authorization its API terms require before Dravr keeps any WHOOP data.
 */
export const PROVIDER_NOTICES: Record<string, ProviderNoticeKeys> = {
  sciotte_trainingpeaks: {
    titleKey: 'providers.trainingpeaksNotice.title',
    bodyKey: 'providers.trainingpeaksNotice.body',
    consentKey: 'providers.trainingpeaksNotice.consent',
  },
  sciotte_coros: {
    titleKey: 'providers.corosNotice.title',
    bodyKey: 'providers.corosNotice.body',
    consentKey: 'providers.corosNotice.consent',
  },
  whoop: {
    titleKey: 'providers.whoopNotice.title',
    bodyKey: 'providers.whoopNotice.body',
    consentKey: 'providers.whoopNotice.consent',
  },
};

/** How the credential login presents one session-read target. */
export interface SciotteLoginPreset {
  /** The provider card (and glyph) id the target is connected through. */
  backend: string;
  /** The provider's name, as the login's header and progress copy say it. */
  labelKey: string;
  /** Title and placeholder of the provider's own credential form. */
  titleKey: string;
  placeholderKey: string;
  /**
   * What the provider signs in with. TrainingPeaks takes a username; a Garmin
   * Connect account takes either, since accounts from before Garmin moved to
   * email sign-in still have a username.
   */
  identifier: LoginIdentifier;
  /** No Google/Apple choice to make: straight to the provider's form. */
  directCredentials: boolean;
  /** The exposure notice shown while the account has not accepted it. */
  notice?: ProviderNoticeKeys;
}

/** What a provider's own login form accepts as the account. */
export type LoginIdentifier = 'email' | 'username' | 'email_or_username';

/** How a credential form's account field presents one kind of identifier. */
export interface LoginIdentifierField {
  /** Corpus key of the field's label. */
  labelKey: string;
  /** The input type: `email` only when nothing but an email is accepted. */
  inputType: 'email' | 'text';
  /** The autocomplete hint, and the field's form name. */
  autoComplete: 'email' | 'username';
}

/**
 * The account field for an identifier kind, read by both clients' login
 * modals. An email-typed field refuses anything without an `@`, so only a
 * provider that accepts nothing else gets one.
 */
export function loginIdentifierField(identifier: LoginIdentifier): LoginIdentifierField {
  switch (identifier) {
    case 'email':
      return { labelKey: 'common.email', inputType: 'email', autoComplete: 'email' };
    case 'username':
      return { labelKey: 'common.username', inputType: 'text', autoComplete: 'username' };
    case 'email_or_username':
      return { labelKey: 'common.emailOrUsername', inputType: 'text', autoComplete: 'username' };
  }
}

/**
 * The login facts of every session-read target, read by both clients' login
 * modals. Each backend is the server's `backend_resolver::hosted_login_target`
 * pairing, which the hosted pages read, so the web and mobile cards open the
 * same login the channel link does.
 */
export const SCIOTTE_LOGIN_PRESETS: Record<SciotteTarget, SciotteLoginPreset> = {
  strava: {
    backend: 'sciotte',
    labelKey: 'shell.sciotteTargetStrava',
    titleKey: 'shell.sciotteStravaAccount',
    placeholderKey: 'shell.stravaEmail',
    identifier: 'email',
    directCredentials: false,
  },
  garmin: {
    backend: 'sciotte_garmin',
    labelKey: 'shell.sciotteProviderGarmin',
    titleKey: 'shell.sciotteGarminAccount',
    placeholderKey: 'shell.garminEmail',
    identifier: 'email_or_username',
    directCredentials: true,
  },
  trainingpeaks: {
    backend: 'sciotte_trainingpeaks',
    labelKey: 'shell.sciotteProviderTrainingPeaks',
    titleKey: 'shell.sciotteTrainingPeaksAccount',
    placeholderKey: 'shell.trainingpeaksUsername',
    identifier: 'username',
    directCredentials: true,
    notice: PROVIDER_NOTICES.sciotte_trainingpeaks,
  },
  coros: {
    backend: 'sciotte_coros',
    labelKey: 'shell.sciotteProviderCoros',
    titleKey: 'shell.sciotteCorosAccount',
    placeholderKey: 'shell.corosEmail',
    identifier: 'email',
    directCredentials: true,
    notice: PROVIDER_NOTICES.sciotte_coros,
  },
};

/**
 * `reason` of a credential sign-in's `otp_required` answer to a code the
 * provider refused or did not take as complete: the same sign-in takes
 * another code. Mirrors the server's `CODE_REJECTED_REASON`.
 */
export const SCIOTTE_CODE_REJECTED = 'code_rejected' as const;

/**
 * Milliseconds between two polls of a number-match sign-in. Google shows the
 * digit until the athlete taps it on their phone, and a poll answers
 * `number_match` at once while it shows, so a client polls again at this
 * pace until the sign-in moves on: connected, a code step or a refusal.
 */
export const SCIOTTE_NUMBER_MATCH_REPOLL_MS = 2000;

/**
 * The `option_id` that leaves a number-match sign-in for the provider's 2FA
 * chooser, for an athlete whose phone never shows the prompt: the step
 * answers `two_factor_choice` (or `otp_required`) on the same flow. Sent only
 * between two polls — a poll in flight holds the flow, and a step sent beside
 * it is refused. Mirrors the sign-in service's `ANOTHER_WAY_OPTION`.
 */
export const SCIOTTE_ANOTHER_WAY_OPTION = 'another_way' as const;

/**
 * `details.reason` of the refusal a credential sign-in step gets once its flow
 * has lapsed: the sign-in has to start again. Mirrors the server's
 * `LOGIN_FLOW_EXPIRED_REASON`; read it with `refusalReason` from
 * `@pierre/ui-logic`.
 */
export const SCIOTTE_LOGIN_FLOW_EXPIRED = 'login_flow_expired' as const;

/**
 * `details.reason` of the refusal a credential sign-in gets when the provider
 * stopped it at a security check (a captcha) the service cannot answer: the
 * athlete retries later. Mirrors the server's `LOGIN_CHALLENGE_REASON`; read
 * it with `refusalReason` from `@pierre/ui-logic`.
 */
export const SCIOTTE_LOGIN_CHALLENGE = 'login_challenge' as const;

/**
 * `details.reason` of the answer a takeover ends with when it ended with no
 * outcome from the service (its socket closed, or it refused the flow).
 * Mirrors the server's `TAKEOVER_ENDED_REASON` (`sciotte_takeover.rs`).
 */
export const SCIOTTE_TAKEOVER_ENDED = 'takeover_ended' as const;

/**
 * The most UTF-8 bytes one takeover `text` input may carry. Mirrors the service's
 * `TAKEOVER_MAX_TEXT_BYTES`; longer text goes as several messages.
 */
export const SCIOTTE_TAKEOVER_MAX_TEXT_BYTES = 1024;

/**
 * The credential-login target for a provider card, or `null` for a provider
 * that connects some other way (OAuth, an API key).
 */
export function sciotteTargetForBackend(provider: string): SciotteTarget | null {
  const entry = (Object.entries(SCIOTTE_LOGIN_PRESETS) as [SciotteTarget, SciotteLoginPreset][]).find(
    ([, preset]) => preset.backend === provider,
  );
  return entry ? entry[0] : null;
}

/**
 * Where a card's history stands, as the card shows it: the server's
 * `history.state` while the card is connected, `not_applicable` otherwise —
 * and for a card from a server that serves no `history`, which shows nothing.
 */
export function cardHistoryState(card: Pick<ExtendedProviderStatus, 'connected' | 'history'>): HistoryState {
  if (!card.connected) return 'not_applicable';
  return card.history?.state ?? 'not_applicable';
}

/**
 * The line a card shows under its name for each history state; a card whose
 * history is `not_applicable` shows none. Each takes `{ provider }`.
 */
export const HISTORY_LINE_KEYS: Readonly<Record<Exclude<HistoryState, 'not_applicable'>, string>> = {
  imported: 'providers.history.imported',
  needed: 'providers.history.needed',
};

/**
 * The words of the card's history action and the line that explains the
 * sign-in it opens (`connectPlan(card).history.signIn`). The action's
 * accessible name is its visible words (`HISTORY_ACTION_KEYS.add`), so
 * `explainer` is only ever a hint or a description, never a label that
 * replaces them (WCAG 2.5.3, label in name).
 */
export const HISTORY_ACTION_KEYS = {
  add: 'providers.history.add',
  explainer: 'providers.history.explainer',
} as const;

/**
 * Whether connecting `provider` must first show its notice: the card says the
 * account has not accepted it, and the client carries a notice to show.
 */
export function noticeRequired(provider: string, consentRequired: boolean | undefined): boolean {
  return Boolean(consentRequired) && provider in PROVIDER_NOTICES;
}

/**
 * The providers whose notice also gates what health sync keeps: while it is
 * owed, sync stores none of the account's records from that provider (WHOOP's
 * owner authorization). A connected card for one of them that owes its notice
 * is not healthy, whatever `connected` says.
 */
const SYNC_GATING_NOTICES: ReadonlySet<string> = new Set(['whoop']);

/**
 * Whether a connected provider has stopped syncing until the athlete accepts
 * its notice: the card is connected, says the notice is owed, and the notice
 * gates what sync keeps. The card then asks for the authorization, and
 * accepting it restarts the provider's connect with the acceptance.
 */
export function syncAuthorizationOwed(
  provider: string,
  connected: boolean,
  consentRequired: boolean | undefined,
): boolean {
  return connected && Boolean(consentRequired) && SYNC_GATING_NOTICES.has(provider);
}

/**
 * The catalogue keys of the consent-to-AI-use controls both clients show in
 * the privacy settings and on the connection card (carnet#726).
 */
export const AI_CONSENT_KEYS = {
  title: 'providers.aiConsent.title',
  blurb: 'providers.aiConsent.blurb',
  label: 'providers.aiConsent.label',
  withdraw: 'providers.aiConsent.withdraw',
  withdrawn: 'providers.aiConsent.withdrawn',
  allowed: 'providers.aiConsent.allowed',
  failed: 'providers.aiConsent.failed',
} as const;

/**
 * The consent class of the athlete's health records (their medical screening
 * answers, injuries, lab results, clinical records, blood glucose): a consent
 * to AI use with no provider card, read and changed through the same
 * `ai-consent` endpoints as a provider's. Dravr's own agent reads them only
 * under it (carnet#1544).
 */
export const SENSITIVE_T2_CONSENT = 'sensitive_t2';

/**
 * The consent class of the athlete's intimate health records (cycle
 * tracking, mental and reproductive health), their notes and their agent's
 * memory: Dravr's own agent and the AI apps the athlete connects (Claude,
 * ChatGPT) read them only under it (carnet#1544).
 */
export const SENSITIVE_T3_CONSENT = 'sensitive_t3';

/**
 * The privacy settings' Sensitive health records block, shown to every
 * account. Its copy names Dravr's own agent, which both switches now govern
 * (carnet#1544).
 */
export const SENSITIVE_CONSENT_KEYS = {
  title: 'providers.aiConsent.sensitive.title',
  blurb: 'providers.aiConsent.sensitive.agentBlurb',
  t2Label: 'providers.aiConsent.sensitive.agentT2Label',
  t3Label: 'providers.aiConsent.sensitive.agentT3Label',
  t3Hint: 'providers.aiConsent.sensitive.agentT3Hint',
  allowed: 'providers.aiConsent.sensitive.allowed',
  withdrawn: 'providers.aiConsent.sensitive.withdrawn',
} as const;

/**
 * The one-time question asking the athlete to let their agent read its
 * memory (`sensitive_t3`) and their health records (`sensitive_t2`), shown
 * once in Dravr's app until answered (carnet#1544).
 */
export const AGENT_CONSENT_PROMPT_KEYS = {
  title: 'providers.aiConsent.agentPrompt.title',
  body: 'providers.aiConsent.agentPrompt.body',
  memoryLabel: 'providers.aiConsent.agentPrompt.memoryLabel',
  medicalLabel: 'providers.aiConsent.agentPrompt.medicalLabel',
  save: 'providers.aiConsent.agentPrompt.save',
  notNow: 'providers.aiConsent.agentPrompt.notNow',
  saved: 'providers.aiConsent.agentPrompt.saved',
  failed: 'providers.aiConsent.agentPrompt.failed',
} as const;

/**
 * The key of Garmin's consent to AI use, as the server records it: one
 * consent for the official API and the sign-in alike (carnet#1157).
 */
export const GARMIN_AI_CONSENT = 'garmin';

/**
 * The key of the athlete's consent to Dravr sending their planned workouts to
 * the Garmin Connect calendar (Garmin's agreement §4.5, carnet#1390).
 */
export const GARMIN_WORKOUT_TRANSFER_CONSENT = 'garmin_workout_transfer';

/** Garmin's privacy policy, which the transfer box links to (§4.5). */
export const GARMIN_PRIVACY_URL = 'https://www.garmin.com/privacy/connect';

/** The catalogue keys of the Garmin connect's two consent boxes. */
export const GARMIN_CONSENT_KEYS = {
  title: 'providers.garminConsent.title',
  intro: 'providers.garminConsent.intro',
  aiLabel: 'providers.garminConsent.aiLabel',
  aiHint: 'providers.garminConsent.aiHint',
  transferLabel: 'providers.garminConsent.transferLabel',
  transferHint: 'providers.garminConsent.transferHint',
  privacyLink: 'providers.garminConsent.privacyLink',
  continue: 'providers.garminConsent.continue',
} as const;

/** One box a connect offers without requiring it. */
export interface ConnectConsent {
  /** The key the server records the consent under, sent in `consents`. */
  key: string;
  labelKey: string;
  hintKey: string;
  /** A policy the box links to, beside its hint. */
  linkUrl?: string;
  linkKey?: string;
}

const GARMIN_CONNECT_CONSENTS: readonly ConnectConsent[] = [
  {
    key: GARMIN_AI_CONSENT,
    labelKey: GARMIN_CONSENT_KEYS.aiLabel,
    hintKey: GARMIN_CONSENT_KEYS.aiHint,
  },
  {
    key: GARMIN_WORKOUT_TRANSFER_CONSENT,
    labelKey: GARMIN_CONSENT_KEYS.transferLabel,
    hintKey: GARMIN_CONSENT_KEYS.transferHint,
    linkUrl: GARMIN_PRIVACY_URL,
    linkKey: GARMIN_CONSENT_KEYS.privacyLink,
  },
];

/**
 * The boxes a connect offers without requiring them, by the card or backend
 * id (`offered_at_connect` in
 * `crates/pierre-core/src/constants/oauth/providers.rs`). Garmin's two: AI
 * use of its data and sending planned workouts to its calendar. Both start
 * unticked and the connect goes through with neither (JF 2026-11-23).
 */
export const CONNECT_CONSENTS: Readonly<Record<string, readonly ConnectConsent[]>> = {
  garmin: GARMIN_CONNECT_CONSENTS,
  sciotte_garmin: GARMIN_CONNECT_CONSENTS,
};

/** The boxes a connect to `provider` offers, or none. */
export function connectConsentsFor(provider: string): readonly ConnectConsent[] {
  return CONNECT_CONSENTS[provider] ?? [];
}

/**
 * The catalogue keys of the transfer-consent switch in the privacy settings
 * (carnet#1390).
 */
export const TRANSFER_CONSENT_KEYS = {
  title: 'providers.transferConsent.title',
  blurb: 'providers.transferConsent.blurb',
  label: 'providers.transferConsent.label',
  allowed: 'providers.transferConsent.allowed',
  withdrawn: 'providers.transferConsent.withdrawn',
  failed: 'providers.transferConsent.failed',
} as const;

/** The fields of a provider card the AI-consent controls read. */
export interface AiConsentCard {
  provider: string;
  connected: boolean;
  ai_consent?: boolean;
}

/** The fields of a provider card the transfer-consent controls read. */
export interface TransferConsentCard {
  provider: string;
  connected: boolean;
  transfer_consent?: boolean;
}

/**
 * The cards whose transfer consent the athlete can give or withdraw: the
 * connected providers whose card carries `transfer_consent` (Garmin).
 */
export function transferConsentCards<T extends TransferConsentCard>(cards: readonly T[]): T[] {
  return cards.filter((card) => card.connected && typeof card.transfer_consent === 'boolean');
}

/**
 * The cards whose consent to AI use the athlete can give or withdraw: the
 * connected providers whose card carries `ai_consent` (WHOOP's owner
 * authorization). A provider asking no such consent is never listed.
 */
export function aiConsentCards<T extends AiConsentCard>(cards: readonly T[]): T[] {
  return cards.filter((card) => card.connected && typeof card.ai_consent === 'boolean');
}

/** The fields of a provider card the Intervals.icu connect path reads. */
export type IntervalsConnectCard = Pick<
  ExtendedProviderStatus,
  'requires_oauth' | 'own_app_required' | 'connected' | 'connection_type'
>;

/**
 * How an Intervals.icu card connects, or reconnects:
 * - `'key'`: the API-key form alone. A card linked by key reconnects through
 *   it, since a coach's key reads the athletes they coach and an
 *   authorization cannot replace it; and with no client to authorize under
 *   (`own_app_required`), the key is the only way in.
 * - `'authorize'`: straight to the authorization, for a card the athlete
 *   authorized.
 * - `'choose'`: a new link while the authorization is offered — the dialog
 *   leads with it and keeps the coach's key form beside it.
 */
export type IntervalsConnectPath = 'authorize' | 'choose' | 'key';

/** The way `card` (the `intervals_icu` card) connects or reconnects. */
export function intervalsConnectPath(card: IntervalsConnectCard): IntervalsConnectPath {
  const authorizeOffered = card.requires_oauth && !card.own_app_required;
  if (!authorizeOffered) return 'key';
  if (card.connected && card.connection_type === 'manual') return 'key';
  if (card.connected && card.connection_type === 'oauth') return 'authorize';
  return 'choose';
}
