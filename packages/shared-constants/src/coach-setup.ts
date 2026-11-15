// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pending coach's Home — who lands on it, whether they train too, and the words of « Préparer ton coaching »
// ABOUTME: Web and phone route on these and read the checklist from GET /api/me/coach-setup, so both draw the same Home (carnet#1529)

import { canRequestCoachAccessWithoutGroup } from './coach-seat';
import { athleteStepsWaived, type PersistedOnboardingStep } from './onboarding';

/**
 * Whether Home is coach-shaped while the coach waits for their access: the
 * user said they coach others and holds no coach access yet (carnet#1529).
 * The same people the group-less request is offered to, so the request is
 * always on the Home they land on. Once a grant gives them coach access, Home
 * is their coach Home.
 */
export function landsOnPendingCoachHome(inputs: { managesRoster: boolean; coachesOthers: boolean }): boolean {
  return canRequestCoachAccessWithoutGroup(inputs);
}

/**
 * Whether a coach trains too, from the onboarding answer the server holds: a
 * coach who answered « J'entraîne d'autres personnes » alone has the athlete
 * steps waived (`not_applicable` rows, which the chat intake writes too); one
 * who answered « J'entraîne et je m'entraîne » does not. A coach who trains
 * keeps their own Home — Getting started, the provider nudge — behind the
 * « Mes athlètes / Mon entraînement » switch; one who does not meets no
 * athlete nudge at all.
 */
export function coachTrainsToo(steps: PersistedOnboardingStep[] | undefined): boolean {
  return !athleteStepsWaived(steps);
}

/**
 * The words of the pending coach's Home, every one resolved with `t()`. New
 * words sit under `humanCoach.pendingHome.*` (ADR-026: they name the human
 * coach's own access and Home); the rest are the existing keys whose meaning
 * is the same here.
 */
export const COACH_SETUP_KEYS = {
  /** The header's scope line before a request. */
  scope: 'humanCoach.pendingHome.scope',
  /** The header's scope line while the request waits. */
  scopePending: 'humanCoach.pendingHome.scopePending',
  /** « Préparer ton coaching ». */
  title: 'humanCoach.pendingHome.title',
  /** « 1 sur 3 », and its spoken form. */
  count: 'home.gettingStarted.count',
  countAria: 'home.gettingStarted.countAria',
  /** A done row's suffix « · fait », read aloud with the row's title. */
  done: 'humanCoach.seat.stepDone',
  /** The invite row. */
  inviteTitle: 'humanCoach.pendingHome.inviteTitle',
  inviteDone: 'humanCoach.pendingHome.inviteDone',
  inviteDoneNoName: 'humanCoach.pendingHome.inviteDoneNoName',
  /** The invite row with no group yet: what a group is, then its athlete and Create. */
  groupIntro: 'humanCoach.groupStepIntro',
  athleteLabel: 'onboarding.groupAthleteLabel',
  athletePlaceholder: 'onboarding.groupAthletePlaceholder',
  create: 'onboarding.groupCreate',
  creating: 'onboarding.groupCreating',
  createFailed: 'onboarding.groupCreateFailed',
  /** The invite row with a group and a link to send. */
  linkIntro: 'onboarding.groupInviteLinkIntro',
  copyLink: 'onboarding.groupCopyLink',
  linkCopied: 'onboarding.groupLinkCopied',
  shareLink: 'onboarding.groupShareLink',
  /** The coaching-platform row. */
  platformTitle: 'humanCoach.pendingHome.platformTitle',
  platformBody: 'humanCoach.pendingHome.platformBody',
  platformDone: 'humanCoach.pendingHome.platformDone',
  /** The groups section under the checklist. */
  groupsTitleOne: 'humanCoach.pendingHome.groupsTitleOne',
  groupsTitleN: 'humanCoach.pendingHome.groupsTitleN',
  groupsNote: 'humanCoach.pendingHome.groupsNote',
  /** The checklist could not be read. */
  loadFailed: 'humanCoach.pendingHome.loadFailed',
} as const;
