// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coaching-platform link vocabulary both group-info surfaces print — each refusal and consent as a corpus key
// ABOUTME: A constants module cannot translate, so it names the keys and the platform each client passes to its own t()

import { PROVIDER_BRAND } from './brands';
import type {
  CoachConsentsResponse,
  CoachPlatformProvider,
  ConfirmDelegatedConnectionRequest,
  DelegationRefusalReason,
  WellnessReadSkip,
} from '@pierre/shared-types';

/**
 * The corpus key each link refusal reads as. The server's message is English
 * and addressed to an API caller; the clients branch on `details.reason`
 * instead and say it in the athlete's language.
 */
export const DELEGATION_REFUSAL_KEY: Record<DelegationRefusalReason, string> = {
  coach_platform_not_connected: 'delegation.connectFirst',
  coach_platform_not_coach_account: 'delegation.notCoachAccount',
  coach_platform_email_missing: 'delegation.coachEmailMissing',
  coach_platform_email_mismatch: 'delegation.coachEmailMismatch',
  dravr_email_unverified: 'delegation.emailUnverified',
  coach_platform_reconnect_needed: 'delegation.reconnectFirst',
  coach_platform_api_key_required: 'delegation.apiKeyRequired',
  coach_platform_terms_outdated: 'delegation.termsOutdated',
  unsupported_provider: 'delegation.actionFailed',
  invalid_athlete: 'delegation.actionFailed',
  athlete_not_on_roster: 'delegation.notOnRoster',
  athlete_no_access: 'delegation.athleteNoAccess',
  athlete_email_missing: 'delegation.athleteEmailMissing',
  athlete_email_mismatch: 'delegation.athleteEmailMismatch',
  member_email_unverified: 'delegation.memberEmailUnverified',
  member_is_coach: 'delegation.actionFailed',
  already_proposed: 'delegation.alreadyProposed',
  athlete_already_linked: 'delegation.athleteAlreadyLinked',
  own_connection: 'delegation.ownConnectionConflict',
  already_linked: 'delegation.alreadyLinked',
  // A grant asked for outside Dravr's own app; both apps are Dravr's own, so
  // neither meets it.
  consent_grant_needs_app: 'delegation.actionFailed',
};

/** Said when a step failed for a reason the table does not name. */
export const DELEGATION_ACTION_FAILED_KEY = 'delegation.actionFailed';

/**
 * The roster refusals the coach resolves on their own coaching-platform
 * connection — connecting it, reconnecting it (with an API key, for
 * Intervals.icu), or connecting the account registered with their Dravr
 * email — so the section offers a way to the connections screen.
 */
export const DELEGATION_CONNECTION_REFUSALS: ReadonlySet<DelegationRefusalReason> = new Set([
  'coach_platform_not_connected',
  'coach_platform_email_missing',
  'coach_platform_email_mismatch',
  'coach_platform_reconnect_needed',
  'coach_platform_api_key_required',
  'coach_platform_terms_outdated',
]);

/**
 * Each coaching platform's brand, the `{{platform}}` every `delegation.*`
 * string names. A brand is a name, not a word, so it reads the same in every
 * language.
 */
const COACH_PLATFORM_NAMES: Record<CoachPlatformProvider, string> = {
  trainingpeaks: PROVIDER_BRAND.trainingpeaks,
  intervals_icu: PROVIDER_BRAND.intervals_icu,
};

/** The backend a platform's own connection is stored under, where it differs from the platform. */
const COACH_PLATFORM_BACKENDS: Record<string, CoachPlatformProvider> = {
  sciotte_trainingpeaks: 'trainingpeaks',
};

/**
 * What `{{platform}}` reads as for `provider` — a roster's or a link's
 * `provider`, a refusal's `details.provider`, or a provider card's backend.
 * Before any platform is known (the roster is loading, or the coach has
 * connected none), it names every platform a coach can connect.
 */
export function coachPlatformName(provider: unknown): string {
  // Own keys only: an index would also find `toString` and the other names a
  // table inherits.
  const own = (table: object, key: string) => Object.prototype.hasOwnProperty.call(table, key);
  if (typeof provider === 'string') {
    if (own(COACH_PLATFORM_NAMES, provider)) {
      return COACH_PLATFORM_NAMES[provider as CoachPlatformProvider];
    }
    if (own(COACH_PLATFORM_BACKENDS, provider)) {
      return COACH_PLATFORM_NAMES[COACH_PLATFORM_BACKENDS[provider]];
    }
  }
  return Object.values(COACH_PLATFORM_NAMES).join(' / ');
}

/** Whether `reason` is one the server sends for a link step. */
export function isDelegationRefusal(reason: unknown): reason is DelegationRefusalReason {
  // Own keys only: `in` would also accept `toString` and the other names the
  // table inherits.
  return typeof reason === 'string' && Object.prototype.hasOwnProperty.call(DELEGATION_REFUSAL_KEY, reason);
}

/** The corpus key a refusal reads as, or the generic failure for anything else. */
export function delegationRefusalKey(reason: unknown): string {
  return isDelegationRefusal(reason) ? DELEGATION_REFUSAL_KEY[reason] : DELEGATION_ACTION_FAILED_KEY;
}

// ============================================================================
// What a member lets their coach do beyond the read
// ============================================================================

/**
 * What a member lets their group's coach do beyond reading their workouts,
 * each the member's alone to grant and revoke, held on their coach
 * relationship rather than on a link: the coach putting workouts on their
 * calendar, their wellness syncing through the coach's account.
 */
export type DelegationConsent = 'calendar_write' | 'wellness_read';

/**
 * The consent fields both the member's coach consents and each link (which
 * mirrors them) carry: what is granted, what is offered, and why the
 * wellness syncs nothing.
 */
type LinkConsentFields = Pick<
  CoachConsentsResponse,
  'calendar_write' | 'calendar_write_supported' | 'wellness_read' | 'wellness_read_supported' | 'wellness_read_skipped'
>;

/**
 * The coaching platform a coach writes an athlete's calendar on through
 * Dravr, which the member's calendar consent names: Intervals.icu, the one
 * platform whose coach account writes an athlete's calendar.
 */
export const COACH_CALENDAR_PLATFORM: CoachPlatformProvider = 'intervals_icu';

/**
 * The words the member's "what your human coach can do" block reads as: its title,
 * the line under it (naming `{{coach}}`), and the step the athlete takes on
 * the platform itself for the calendar consent to work (naming `{{coach}}`
 * and `{{platform}}`).
 */
export const COACH_CONSENTS_KEYS = {
  title: 'delegation.coachConsentsTitle',
  hint: 'delegation.coachConsentsHint',
  calendarStep: 'humanCoach.calendarWriteStep',
} as const;

/**
 * The words each consent reads as: the member's control and its hint, both
 * naming `{{coach}}` and `{{platform}}`, and the state line the coach reads
 * once it is granted.
 */
export const DELEGATION_CONSENT_KEYS: Record<DelegationConsent, { label: string; hint: string; granted: string }> = {
  calendar_write: {
    label: 'delegation.calendarWriteLabel',
    hint: 'delegation.calendarWriteHint',
    granted: 'delegation.calendarWriteGranted',
  },
  wellness_read: {
    label: 'delegation.wellnessReadLabel',
    hint: 'delegation.wellnessReadHint',
    granted: 'delegation.wellnessReadGranted',
  },
};

/** The coach's state line for a link whose member granted none of what it offers. */
export const DELEGATION_CONSENTS_NONE_KEY = 'delegation.consentsWorkoutsOnly';

/**
 * The words a link whose wellness syncs nothing reads as, for each reason:
 * the coach's state line once the member granted it, in place of "shares",
 * and the note under the member's control. A member's own connection to the
 * link's platform has no coach line of its own, so the coach reads no
 * wellness line at all rather than one that is not true.
 */
export const WELLNESS_READ_SKIP_KEYS: Record<WellnessReadSkip, { state: string | null; note: string }> = {
  own_wellness_source: {
    state: 'delegation.wellnessReadOwnSource',
    note: 'delegation.wellnessReadOwnSourceNote',
  },
  own_connection: {
    state: null,
    note: 'delegation.ownConnectionConflict',
  },
};

/**
 * The note under the member's control for `consent` on `link`: why it syncs
 * nothing even when granted, or `undefined` when nothing keeps it from
 * syncing.
 */
export function consentNoteKey(link: LinkConsentFields, consent: DelegationConsent): string | undefined {
  if (consent !== 'wellness_read' || !link.wellness_read_skipped) return undefined;
  return WELLNESS_READ_SKIP_KEYS[link.wellness_read_skipped].note;
}

/** The consents `link` (a link, or the member's coach consents) offers, in the order both surfaces list them. */
export function offeredConsents(link: LinkConsentFields): DelegationConsent[] {
  const offered: DelegationConsent[] = [];
  if (link.calendar_write_supported) offered.push('calendar_write');
  if (link.wellness_read_supported) offered.push('wellness_read');
  return offered;
}

/**
 * The keys the coach's state line for a confirmed `link` reads as: each
 * consent its member granted, or "workouts only" for none. A granted
 * wellness consent that syncs nothing reads as why ({@link WELLNESS_READ_SKIP_KEYS})
 * instead of "shares". A platform that offers no consent has no line, since
 * there is nothing for the member to have granted.
 */
export function consentStateKeys(link: LinkConsentFields): string[] {
  const offered = offeredConsents(link);
  if (offered.length === 0) return [];
  const keys = offered
    .filter((consent) => link[consent])
    .map((consent) => {
      if (consent === 'wellness_read' && link.wellness_read_skipped) {
        return WELLNESS_READ_SKIP_KEYS[link.wellness_read_skipped].state;
      }
      return DELEGATION_CONSENT_KEYS[consent].granted;
    })
    .filter((key): key is string => key !== null);
  return keys.length > 0 ? keys : [DELEGATION_CONSENTS_NONE_KEY];
}

/**
 * What the member's confirm of `link` sends: an answer, granted or not, for
 * each consent it offers. A link that offers none is confirmed with no body,
 * which grants nothing.
 */
export function confirmConsents(
  link: LinkConsentFields,
  granted: Partial<Record<DelegationConsent, boolean>>,
): ConfirmDelegatedConnectionRequest | undefined {
  const offered = offeredConsents(link);
  if (offered.length === 0) return undefined;
  const request: ConfirmDelegatedConnectionRequest = {};
  for (const consent of offered) request[consent] = granted[consent] === true;
  return request;
}

/** The words a confirm a consent change asks first reads as. */
export interface ConsentChangeConfirmKeys {
  title: string;
  /** Names `{{platform}}` and `{{coach}}`. */
  body: string;
  confirm: string;
}

/**
 * The confirm a revoke asks first, for each consent whose revoke deletes what
 * the link already read: turning wellness off removes the wellness Dravr read
 * through the coach's account. The cancel is each app's own. A grant needs no
 * confirm, nor does a revoke of a consent not named here.
 */
export const DELEGATION_REVOKE_CONFIRM_KEYS: Partial<Record<DelegationConsent, ConsentChangeConfirmKeys>> = {
  wellness_read: {
    title: 'delegation.wellnessRevokeTitle',
    body: 'delegation.wellnessRevokeBody',
    confirm: 'delegation.wellnessRevokeConfirm',
  },
};

/** The confirm turning `consent` to `granted` asks first, or `undefined` when it asks none. */
export function consentChangeConfirm(
  consent: DelegationConsent,
  granted: boolean,
): ConsentChangeConfirmKeys | undefined {
  return granted ? undefined : DELEGATION_REVOKE_CONFIRM_KEYS[consent];
}
