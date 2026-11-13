// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach seat — who may ask for coach access, where their request stands, what the onboarding seat step says, and its words
// ABOUTME: The room strip, the thread header, Group info and the seat step read these on web and phone, from server state alone

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
  /** The viewer's membership in it is the owner's. */
  isOwner: boolean;
  /** The viewer already holds coach access (`manages_roster`). */
  managesRoster: boolean;
  /** The viewer answered "I coach others" at onboarding. */
  coachesOthers: boolean;
}

/**
 * Whether this viewer is offered the coach seat for this group: they own it,
 * it has no human coach, they hold no coach access, and they coach others.
 *
 * The one place the rule lives. Every surface that offers the request — the
 * room strip, the thread header, the Group info block, the onboarding invite,
 * on web and phone — calls this, so changing who may ask is a change here and
 * nowhere else.
 */
export function canRequestCoachAccess({ group, isOwner, managesRoster, coachesOthers }: CoachSeatInputs): boolean {
  return isOwner && !!group && !group.coach_user_id && !managesRoster && coachesOthers;
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
  pendingTitle: 'humanCoach.seat.pendingTitle',
  pendingBody: 'humanCoach.seat.pendingBody',
  pendingBlockTitle: 'humanCoach.seat.pendingBlockTitle',
  timelineLabel: 'humanCoach.seat.timelineLabel',
  stepSent: 'humanCoach.seat.stepSent',
  stepSentWhen: 'humanCoach.seat.stepSentWhen',
  stepReview: 'humanCoach.seat.stepReview',
  stepReviewWhen: 'humanCoach.seat.stepReviewWhen',
  stepSeat: 'humanCoach.seat.stepSeat',
  stepSeatWhat: 'humanCoach.seat.stepSeatWhat',
  stepDone: 'humanCoach.seat.stepDone',
  stepNow: 'humanCoach.seat.stepNow',
  declinedTitle: 'humanCoach.seat.declinedTitle',
  declinedBody: 'humanCoach.seat.declinedBody',
  askAgain: 'humanCoach.seat.askAgain',
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
