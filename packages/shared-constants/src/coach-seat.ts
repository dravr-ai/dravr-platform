// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach seat — who may ask for coach access, where their request stands, what the onboarding seat step and the list row say, and its words
// ABOUTME: The room strip, the thread header, Group info, Settings, Home, the Groups list and the seat step read these on web and phone, from server state alone

import { formatDate, formatMessageTime } from '@pierre/chat-utils';
import type { CoachAccessRequest, CoachingGroup } from '@pierre/shared-types';
import { COACH_PLATFORM_PROVIDERS } from './onboarding';

/**
 * Where the viewer's coach seat stands, read from their latest coach-access
 * request (carnet#738). Asking grants nothing (ADR-018): a super-admin
 * decides, so the seat only ever waits on a person.
 *
 * - `to_request`: never asked, or the last request was granted and the access
 *   has since gone (an operator revoked it). Nothing is said to be pending.
 * - `pending`: the request is with the Dravr team.
 * - `declined`: the team said no; the coach may ask again.
 */
export type CoachSeatState = 'to_request' | 'pending' | 'declined';

/** What the seat rule reads about the viewer and the group. */
export interface CoachSeatInputs {
  /** The group the viewer is looking at, once read. */
  group: Pick<CoachingGroup, 'coach_user_id'> | null | undefined;
  /** The viewer owns the group (`owner_id`): the only one a grant attaches. */
  isOwner: boolean;
  /** The viewer already holds coach access (`manages_roster`). */
  managesRoster: boolean;
}

/**
 * Whether this viewer is offered the coach seat for this group: they own it,
 * it has no human coach, and they hold no coach access (carnet#1522).
 *
 * What they answered to "I coach others" at onboarding plays no part: an
 * owner who started as an athlete and coaches later is offered it all the
 * same, and a member who does not own the group never is — a grant attaches
 * the coach to the coachless groups they own, so nobody else has a seat to
 * ask for.
 *
 * The one place the rule lives. Every surface that offers the request — the
 * room strip, the thread header, the Group info block, the onboarding invite,
 * on web and phone — calls this, so changing who may ask is a change here and
 * nowhere else.
 */
export function canRequestCoachAccess({ group, isOwner, managesRoster }: CoachSeatInputs): boolean {
  return isOwner && !!group && !group.coach_user_id && !managesRoster;
}

/**
 * Whether the request is offered with no group, in Settings and on Home: to
 * someone who said they coach others and holds no coach access yet. A coach
 * who taps "Later" on the onboarding group step still has their request
 * there, and a grant attaches them to every coachless group they make or own
 * (carnet#1521).
 */
export function canRequestCoachAccessWithoutGroup({
  managesRoster,
  coachesOthers,
}: {
  managesRoster: boolean;
  coachesOthers: boolean;
}): boolean {
  return coachesOthers && !managesRoster;
}

/**
 * Where the seat stands for the latest request the server returned. A granted
 * request whose access the viewer no longer holds reads as `to_request`: the
 * rule above only offers the seat to someone without coach access.
 */
export function coachSeatState(request: CoachAccessRequest | null | undefined): CoachSeatState {
  if (request?.status === 'pending') return 'pending';
  if (request?.status === 'declined') return 'declined';
  return 'to_request';
}

/** The words of the coach seat, every one under `humanCoach.*` (ADR-026). */
export const COACH_SEAT_KEYS = {
  title: 'humanCoach.seat.title',
  stripBody: 'humanCoach.seat.stripBody',
  blockBody: 'humanCoach.seat.blockBody',
  linkAccount: 'humanCoach.seat.linkAccount',
  routesLabel: 'humanCoach.seat.routesLabel',
  pendingSentBody: 'humanCoach.seat.pendingSentBody',
  pendingBlockTitle: 'humanCoach.seat.pendingBlockTitle',
  timelineLabel: 'humanCoach.seat.timelineLabel',
  stepSent: 'humanCoach.seat.stepSent',
  stepSentWhen: 'humanCoach.seat.stepSentWhen',
  stepReview: 'humanCoach.seat.stepReview',
  stepReviewNotice: 'humanCoach.seat.stepReviewNotice',
  stepSeat: 'humanCoach.seat.stepSeat',
  stepSeatWhat: 'humanCoach.seat.stepSeatWhat',
  stepDone: 'humanCoach.seat.stepDone',
  stepNow: 'humanCoach.seat.stepNow',
  declinedTitle: 'humanCoach.seat.declinedTitle',
  declinedBody: 'humanCoach.seat.declinedBody',
  declinedReason: 'humanCoach.seat.declinedReason',
  askAgain: 'humanCoach.seat.askAgain',
  cancel: 'humanCoach.seat.cancel',
  cancelling: 'humanCoach.seat.cancelling',
  cancelled: 'humanCoach.seat.cancelled',
  cancelFailed: 'humanCoach.seat.cancelFailed',
  panelTitle: 'humanCoach.seat.panelTitle',
  panelBody: 'humanCoach.seat.panelBody',
  settingsTitle: 'humanCoach.seat.settingsTitle',
  listPending: 'humanCoach.seat.listPending',
  listCoach: 'humanCoach.seat.listCoach',
  grantedTitle: 'humanCoach.access.grantedTitle',
  grantedBody: 'humanCoach.access.grantedBody',
  openHome: 'humanCoach.access.openHome',
  rosterNotYet: 'humanCoach.seat.rosterNotYet',
  rosterPending: 'humanCoach.seat.rosterPending',
  request: 'humanCoach.requestAccess',
  sending: 'humanCoach.requestAccessSending',
  failed: 'humanCoach.requestAccessFailed',
} as const;

/** The thread header's state word for each seat state: what the coach can do, or what they wait on. */
export const COACH_SEAT_SUBTITLE_KEY: Record<CoachSeatState, string> = {
  to_request: 'humanCoach.seat.subtitleToRequest',
  pending: 'humanCoach.seat.subtitlePending',
  declined: 'humanCoach.seat.subtitleDeclined',
};

/**
 * The reason a declined request carries, trimmed; `null` when the
 * super-admin gave none (or the request is not declined).
 */
export function coachSeatDeclineReason(request: CoachAccessRequest | null | undefined): string | null {
  if (request?.status !== 'declined') return null;
  const reason = request.decline_reason?.trim();
  return reason ? reason : null;
}

/**
 * How long after a grant a room the coach now coaches says so on its list
 * row, unless something was said in it since: the grant's own line lands in
 * the room a moment after the decision.
 */
export const COACH_SEAT_LIST_GRANT_GRACE_MS = 2 * 60 * 1000;

/**
 * The second line a Groups list row shows instead of the room's last words,
 * for a room the viewer owns while their coach seat is moving:
 *   - `pending` — the room has no coach and the viewer's request waits, sent
 *     on `sentAt`: « Accès coach en attente · demandé le … ». The request is
 *     one per account, so every coachless room they own says the same.
 *   - `coach` — the viewer coaches the room because a grant decided at
 *     `since` attached them, and nothing was said in the room after the
 *     grant's own line: « Tu es le coach · depuis … ». Once the room talks
 *     again, its last words come back.
 * `null` for every other room, and whenever the request is not read yet.
 */
export type CoachSeatListLine = { kind: 'pending'; sentAt: string } | { kind: 'coach'; since: string };

export function coachSeatListLine({
  group,
  viewerId,
  managesRoster,
  request,
  lastActivityAt,
}: {
  group: Pick<CoachingGroup, 'owner_id' | 'coach_user_id'> | null | undefined;
  viewerId: string | null | undefined;
  managesRoster: boolean;
  request: CoachAccessRequest | null | undefined;
  lastActivityAt: string;
}): CoachSeatListLine | null {
  if (!group || !viewerId || group.owner_id !== viewerId || !request) return null;
  if (request.status === 'pending' && !group.coach_user_id && !managesRoster) {
    return { kind: 'pending', sentAt: request.created_at };
  }
  if (request.status === 'granted' && request.decided_at && group.coach_user_id === viewerId) {
    const decided = Date.parse(request.decided_at);
    const last = Date.parse(lastActivityAt);
    if (!Number.isNaN(decided) && (Number.isNaN(last) || last <= decided + COACH_SEAT_LIST_GRANT_GRACE_MS)) {
      return { kind: 'coach', since: request.decided_at };
    }
  }
  return null;
}

/**
 * The day a request was filed or decided, as the reader's locale spells it
 * (`12 nov. 2026`), and the time on the bubbles' 24-hour clock (`14:32`).
 * An unparseable stamp comes back verbatim from the day and empty from the
 * time, as the formatters they wrap do.
 */
export function coachSeatMoment(iso: string, language: string): { date: string; time: string } {
  return { date: formatDate(iso, language), time: formatMessageTime(iso, language) };
}

/**
 * How long the seat is read again, after a coaching platform is first seen
 * connected, for that connection to settle it. Both outcomes are written off
 * the connect request: a TrainingPeaks coach account's `manages_roster` once
 * its role is read, an Intervals.icu request once its roster is read. The
 * onboarding seat step then says the connection produced nothing; Group
 * info's seat block, where a platform can be linked too, stops re-reading.
 */
export const COACH_SEAT_CHECK_WINDOW_MS = 20_000;

/** How often the seat step and the seat block read the server again while they wait. */
export const COACH_SEAT_CHECK_POLL_MS = 2_500;

/**
 * How often the signed-in app reads a waiting coach-access request again, so
 * a grant shows without signing out and in (carnet#1521), on web and phone.
 */
export const COACH_ACCESS_WAIT_POLL_MS = 60_000;

/**
 * Where the coach's seat stands, as the onboarding seat step shows it. The
 * request half is {@link coachSeatState}; this adds what a connection did:
 *   - `open` — nothing connected, nothing asked: the rows and the link.
 *   - `coach` — the coach holds coach access (`manages_roster`).
 *   - `requested` — a request is waiting, sent at `sentAt` (date it with
 *     {@link coachSeatMoment}).
 *   - `declined` — the latest request was declined; asking again is offered.
 *   - `checking` — a coaching platform is connected and its outcome is not
 *     in yet.
 *   - `nothing` — a coaching platform (`provider`, its display name) is
 *     connected and produced neither a seat nor a request.
 */
export type CoachSeatStepState =
  | { kind: 'open' }
  | { kind: 'coach' }
  | { kind: 'requested'; sentAt: string }
  | { kind: 'declined' }
  | { kind: 'checking' }
  | { kind: 'nothing'; provider: string };

/** The parts of a provider card the seat step reads. */
export interface CoachSeatProvider {
  provider: string;
  display_name: string;
  connected: boolean;
  account_role?: 'athlete' | 'coach';
}

/** The coaching platforms among `providers` that are connected, in seat order. */
export function connectedCoachPlatforms<T extends CoachSeatProvider>(providers: readonly T[]): T[] {
  return COACH_PLATFORM_PROVIDERS.flatMap((name) =>
    providers.filter((p) => p.provider === name && p.connected),
  );
}

/**
 * The seat step as the server has it. Precedence follows what the coach can
 * act on: coach access first, then a request waiting, then a declined one
 * (which no connection reopens: a roster files a request once per account),
 * then a connection still settling or settled with nothing.
 *
 * A TrainingPeaks account read as an athlete's settles at once: only a coach
 * account grants. Anything else waits for `checkExpired`, the end of
 * {@link COACH_SEAT_CHECK_WINDOW_MS}.
 */
export function coachSeatStepState({
  managesRoster,
  request,
  providers,
  checkExpired,
}: {
  managesRoster: boolean;
  request: CoachAccessRequest | null | undefined;
  providers: readonly CoachSeatProvider[];
  checkExpired: boolean;
}): CoachSeatStepState {
  if (managesRoster) return { kind: 'coach' };
  const asked = coachSeatState(request);
  if (request && asked === 'pending') return { kind: 'requested', sentAt: request.created_at };
  if (asked === 'declined') return { kind: 'declined' };
  const connected = connectedCoachPlatforms(providers);
  const first = connected[0];
  if (!first) return { kind: 'open' };
  const onlyAnAthleteAccount = connected.every(
    (p) => p.provider === 'sciotte_trainingpeaks' && p.account_role === 'athlete',
  );
  return checkExpired || onlyAnAthleteAccount
    ? { kind: 'nothing', provider: first.display_name }
    : { kind: 'checking' };
}
