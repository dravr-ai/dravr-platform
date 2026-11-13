// ABOUTME: Declarative onboarding step registry — canonical order + per-step applicability/completion predicates
// ABOUTME: Single source of truth for which onboarding step is current; consumed by BOTH web and mobile

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import activitySports from './activity-sports.json' with { type: 'json' };

/** Identifiers for the onboarding steps, in canonical journey order. */
export type OnboardingStepId =
  | 'analytics_consent'
  | 'profile_type'
  | 'about_you'
  | 'parq'
  | 'check_in'
  | 'connect_provider'
  | 'coach_proposal'
  | 'coach_seat'
  | 'coach_group'
  | 'coach_invite';

/**
 * The signals the flow derives "which step is current" from:
 *   - `onboardingActive` — authenticated, active, non-admin (admins are exempt
 *     from the whole flow).
 *   - `needsProviderConnection` — server truth; `undefined` while the query is
 *     in flight or disabled (e.g. admins).
 *   - `skippedProvider` — session-only "continue without connecting".
 *   - `justOnboarded` — a `needs_provider_connection` true→false transition
 *     observed *this tab*, which keeps the connect step up (connect another
 *     service) until Continue. It gates nothing else: a tab that opens after
 *     the OAuth return never observes the flip (carnet#1042).
 *   - `profileTypeChosen` / `coachProposalDone` — the per-user local flag OR
 *     the server's step record, so a step done on any surface stays done.
 *   - `connectProviderRecorded` — the server's durable record that this user
 *     has had a provider connected, which closes the pre-connect phase for
 *     good (carnet#1341).
 */
export interface OnboardingContext {
  onboardingActive: boolean;
  needsProviderConnection: boolean | undefined;
  /**
   * The server records the connect step complete: this user has had a real
   * provider connected. `needsProviderConnection` is live, so disconnecting
   * the last provider turns it back to `true`; this flag is what keeps an
   * onboarded athlete out of the wizard then (carnet#1341). A user who never
   * connected has no record, so their connect step still comes back at each
   * login — the skip stays session-only on purpose.
   */
  connectProviderRecorded: boolean;
  skippedProvider: boolean;
  /**
   * The steps the user put off: the server's `skipped` rows
   * (`skippedStepIds`), which the surfaces write into their cached status as
   * they skip. Only the progress bar reads it, to draw such a step as passed
   * rather than done; the decision engine never does.
   */
  skippedSteps: readonly OnboardingStepId[];
  justOnboarded: boolean;
  profileTypeChosen: boolean;
  coachProposalDone: boolean;
  /**
   * The about-you answers (North Star / sport / goal) have been given or skipped.
   *
   * These are the inputs `build_coach_proposal` already reads and falls back
   * without, which is why the step sits ahead of the provider gate: provider
   * sync is not something we control, three answered questions are.
   */
  aboutYouDone: boolean;
  /**
   * The user pressed Continue on the connect step after connecting a first
   * provider this session. Until then the step stays on screen, so a second
   * service is one tap away rather than a trip to Settings.
   */
  connectProviderDone: boolean;
  /** The PAR-Q medical screen has been answered or skipped. */
  parqDone: boolean;
  /**
   * How often the coach checks in on its own has been answered or skipped
   * (carnet#831). `off` is an answer; a skip stores no cadence, which reads
   * as off too.
   */
  checkInDone: boolean;
  /**
   * The user coaches others and does not train themselves, so the athlete
   * steps (`about_you`, `parq`) are not part of their journey — neither asked
   * nor shown on the progress bar. Durable as `not_applicable` step rows (see
   * `athleteStepsWaived`), which the chat intake writes too.
   */
  athleteStepsWaived: boolean;
  /**
   * The user answered "I coach others" (server truth: `coaches_others`), so the
   * group step — create the group, share the athlete invite — is theirs.
   */
  coachesOthers: boolean;
  /** The group step has been finished or put off (`complete` / `skipped`). */
  coachGroupDone: boolean;
  /**
   * The coach has moved past the coach-seat step: Continue once a connection
   * or a request settled it, or « Passer pour l'instant ». A per-user device
   * flag; the seat itself is server state the step reads, never this flag.
   */
  coachSeatDone: boolean;
  /**
   * The coach's group exists this session, so its invite is the next screen.
   * Session-only: the invite needs the link the creation just issued.
   */
  coachGroupCreated: boolean;
  /**
   * The user has answered the usage-analytics question, yes or no, on any
   * surface (server truth: `analytics_consent_answered`). An unknown answer
   * (an older server, a cached user) counts as answered, so the question
   * never flashes at someone who already gave it (carnet#1366).
   */
  analyticsConsentAnswered: boolean;
}

/** A single step in the onboarding journey. */
export interface OnboardingStepDef {
  id: OnboardingStepId;
  /** Corpus key of the short label the progress indicator shows; resolved with t() at render. */
  labelKey: string;
  /**
   * Whether this step belongs to *this* user's journey right now. Phase-gated:
   * profile-type and connect-provider belong to the pre-connect phase
   * (`needs === true`); coach-proposal and the coach's group to the
   * post-connect phase (`needs === false`).
   */
  isApplicable: (ctx: OnboardingContext) => boolean;
  /**
   * Whether this step has been satisfied. Drives both the decision engine
   * (step past a complete step) and the progress indicator's "done" state, so
   * it is defined phase-agnostically (see each step).
   */
  isComplete: (ctx: OnboardingContext) => boolean;
  /**
   * A one-time question rather than a stage of the journey: never counted on
   * the progress indicator, which is not drawn while it is asked.
   */
  hiddenFromProgress?: boolean;
}

/**
 * The providers a coach coaches on, in the order the coach-seat step offers
 * them. A TrainingPeaks coach account on the user's verified email is coach
 * access; an Intervals.icu roster that coaches an active athlete files the
 * request. Either account is also how a group's athletes are read (delegated
 * connections) once they join: the TrainingPeaks coach session, or the
 * Intervals.icu coach's API key.
 */
export const COACH_PLATFORM_PROVIDERS: readonly string[] = [
  'sciotte_trainingpeaks',
  'intervals_icu',
];

/**
 * Whether the post-connect steps are open: a provider is connected, one has
 * been (the server's connect-step record — an athlete who disconnects midway
 * still meets the steps after the gate, carnet#1341), or the user is a coach
 * who does not train. Nothing reads such a coach's own
 * training, so a missing provider holds back none of their steps: they meet
 * the coach-seat step instead of the connect step.
 * `undefined` (status still loading) opens nothing, so no step flashes.
 */
export function pastProviderGate(
  c: Pick<
    OnboardingContext,
    'needsProviderConnection' | 'connectProviderRecorded' | 'athleteStepsWaived'
  >,
): boolean {
  if (c.needsProviderConnection === undefined) return false;
  return (
    c.needsProviderConnection === false || c.connectProviderRecorded || c.athleteStepsWaived
  );
}

/**
 * Whether the user is still before the provider gate: no provider now, and
 * none ever recorded. An athlete who connected and later disconnected is past
 * it for good, so no pre-connect step comes back (carnet#1341).
 */
export function beforeProviderGate(
  c: Pick<OnboardingContext, 'needsProviderConnection' | 'connectProviderRecorded'>,
): boolean {
  return c.needsProviderConnection === true && !c.connectProviderRecorded;
}

/**
 * Whether the coach path — seat, group, invite — is open: the user coaches
 * others and is past the provider gate, or chose to continue without a
 * provider. Skipping leads on to these steps rather than past them, so a coach
 * who trains and skips still leaves with a group. A coach who does not train
 * (`athleteStepsWaived`) coaches others by definition, so a status read that
 * has not caught up with the answer yet opens the path all the same.
 */
function coachStepsOpen(
  c: Pick<
    OnboardingContext,
    | 'onboardingActive'
    | 'coachesOthers'
    | 'needsProviderConnection'
    | 'connectProviderRecorded'
    | 'athleteStepsWaived'
    | 'skippedProvider'
  >,
): boolean {
  return (
    c.onboardingActive &&
    (c.coachesOthers || c.athleteStepsWaived) &&
    (pastProviderGate(c) || (c.skippedProvider && c.needsProviderConnection !== undefined))
  );
}

/**
 * Canonical onboarding pipeline, in order. The current step is the first
 * applicable step that is not yet complete (see `currentOnboardingStep`); when
 * none remain, onboarding is done and the dashboard/chat renders.
 */
export const ONBOARDING_STEPS: OnboardingStepDef[] = [
  {
    // The usage-analytics question, for an account no form asked: a Google or
    // Apple sign-in, a sign-up in chat, an account older than the question.
    // The sign-up form asks with its own box, so its accounts arrive answered.
    // First, so the events of everything after it follow the answer; read
    // from server state, so an onboarded account meets it once too.
    id: 'analytics_consent',
    labelKey: 'onboarding.analyticsConsentTitle',
    isApplicable: (c) => c.onboardingActive,
    isComplete: (c) => c.analyticsConsentAnswered,
    hiddenFromProgress: true,
  },
  {
    id: 'profile_type',
    labelKey: 'onboarding.stepProfile',
    isApplicable: (c) => c.onboardingActive && beforeProviderGate(c) && !c.skippedProvider,
    isComplete: (c) => c.profileTypeChosen,
  },
  {
    // Who the athlete is, in three questions. Deliberately BEFORE the provider
    // gate: the coach proposal reads these, and on a first-run connection the
    // provider data it would otherwise infer from has not synced yet — so this
    // is the only proposal input we can guarantee exists when it runs. It also
    // means a user who never connects still gets a non-generic proposal.
    id: 'about_you',
    labelKey: 'onboarding.stepAboutTraining',
    isApplicable: (c) =>
      c.onboardingActive &&
      beforeProviderGate(c) &&
      !c.skippedProvider &&
      !c.athleteStepsWaived,
    isComplete: (c) => c.aboutYouDone,
  },
  {
    // Pre-participation medical screen. Ahead of the provider gate for the same
    // reason it exists at all: an agent should not prescribe load before we have
    // asked. A "yes" raises an agent-visible flag and never blocks sign-up.
    // A coach who does not train is not screened: no load is prescribed to them.
    id: 'parq',
    labelKey: 'onboarding.stepHealthCheck',
    isApplicable: (c) =>
      c.onboardingActive &&
      beforeProviderGate(c) &&
      !c.skippedProvider &&
      !c.athleteStepsWaived,
    isComplete: (c) => c.parqDone,
  },
  {
    // How often the coach writes first: every morning, three a week, weekly or
    // never. Asked with the other questions about the athlete, ahead of the
    // provider gate, and changeable in Settings. A coach who does not train is
    // not asked: a check-in reads the athlete's own training.
    id: 'check_in',
    labelKey: 'onboarding.stepCheckIn',
    isApplicable: (c) =>
      c.onboardingActive &&
      beforeProviderGate(c) &&
      !c.skippedProvider &&
      !c.athleteStepsWaived,
    isComplete: (c) => c.checkInDone,
  },
  {
    // An athlete's own data. A coach who does not train never meets it:
    // nothing reads their own training, and the coaching platforms are offered
    // on the coach-seat step instead, where they decide coach access.
    //
    // The first connection does not close it: a user who has just connected one
    // service (`justOnboarded`) stays on the list to connect another, and moves
    // on with Continue. A returning, already-connected user never sees it, and
    // neither does one who has since disconnected every provider: the server
    // record keeps them past the gate (carnet#1341).
    id: 'connect_provider',
    labelKey: 'onboarding.stepConnect',
    isApplicable: (c) =>
      (beforeProviderGate(c) || (c.justOnboarded && !c.connectProviderDone)) &&
      !c.skippedProvider &&
      !c.athleteStepsWaived,
    // Never "done" for a coach who does not train either: a coaching platform
    // they connect on the seat step is not this step, and counting it would
    // grow their bar by a segment after a reload.
    isComplete: (c) =>
      !c.athleteStepsWaived &&
      c.needsProviderConnection === false &&
      (c.connectProviderDone || !c.justOnboarded),
  },
  {
    // An agent for the athlete's own training, ranked on their own activities.
    // A coach who does not train has no such agent: their group's agent is
    // chosen while creating the group, in the coach_group step.
    //
    // Derived from server state — a provider connected and the step not yet
    // recorded done or declined — never from the tab-local `justOnboarded`
    // flip: the OAuth return often lands in a fresh tab that never sees the
    // flip, and that tab must still meet the proposal (carnet#1042). The
    // tab that did see it still meets the connect step first, which stays up
    // until Continue. `coachProposalDone` reads the server record, so a user
    // who finished it on any surface is not asked again. An athlete who
    // continues without connecting meets it too, offered the agents that need
    // no provider, so nobody reaches Home without a team (carnet#1095).
    id: 'coach_proposal',
    labelKey: 'onboarding.stepAgent',
    isApplicable: (c) =>
      c.onboardingActive &&
      !c.athleteStepsWaived &&
      (c.needsProviderConnection === false || c.connectProviderRecorded || c.skippedProvider),
    isComplete: (c) => c.coachProposalDone,
  },
  {
    // The coach's seat (`manages_roster`) before anything is made: connect a
    // TrainingPeaks coach account (coach access, no human in the loop), an
    // Intervals.icu account (files the request), or ask the Dravr team. Asking
    // grants nothing (ADR-018). Only until the group step is behind the coach,
    // so a coach who onboarded before it existed is not sent back to it; a
    // coach who already holds the seat is stepped past by the screen itself.
    id: 'coach_seat',
    labelKey: 'humanCoach.stepSeat',
    isApplicable: (c) => coachStepsOpen(c) && !c.coachGroupDone,
    isComplete: (c) => c.coachSeatDone,
  },
  {
    // A coach leaves onboarding with a group and an athlete invite link instead
    // of discovering /group create in chat. Its athlete, name and agent are on
    // one screen; the agent is chosen for the group, never inherited from the
    // coach's own. Like the agent step it reads server state, so a coach who
    // onboarded before this step existed is offered it once. Making the group
    // completes this screen; the step is recorded once the invite is behind
    // the coach (or the group put off), which is what `coach_invite` waits on.
    id: 'coach_group',
    labelKey: 'onboarding.stepGroup',
    isApplicable: (c) => coachStepsOpen(c),
    isComplete: (c) => c.coachGroupDone || c.coachGroupCreated,
  },
  {
    // The athlete invite for the group just made: one job, sharing its link.
    // Its own segment on the progress bar; a group put off leaves nothing to
    // share, so `coachGroupDone` completes it too.
    id: 'coach_invite',
    labelKey: 'onboarding.stepInvite',
    isApplicable: (c) => coachStepsOpen(c),
    isComplete: (c) => c.coachGroupDone,
  },
];

/**
 * The current onboarding step: the first applicable step not yet complete, or
 * `null` when onboarding is finished (→ render the dashboard/chat).
 */
export function currentOnboardingStep(ctx: OnboardingContext): OnboardingStepDef | null {
  return firstPendingStep(ONBOARDING_STEPS, ctx) ?? null;
}

/** The first of `steps` that applies to `ctx` and is not yet complete. */
function firstPendingStep(
  steps: readonly OnboardingStepDef[],
  ctx: OnboardingContext,
): OnboardingStepDef | undefined {
  return steps.filter((s) => s.isApplicable(ctx)).find((s) => !s.isComplete(ctx));
}

/** A persisted onboarding step record as returned by the server (structural). */
export interface PersistedOnboardingStep {
  step_id: string;
  status: string;
}

/**
 * Whether the server's durable record marks `stepId` as done (`complete` or
 * `skipped`). This is what makes onboarding survive device changes: a step
 * finished on one device is stepped past on another even with no local flag.
 */
export function isServerStepComplete(
  steps: PersistedOnboardingStep[] | undefined,
  stepId: OnboardingStepId,
): boolean {
  return (steps ?? []).some(
    (s) => s.step_id === stepId && (s.status === 'complete' || s.status === 'skipped'),
  );
}

/**
 * Whether the server has recorded the connect step `complete`: this user has
 * had a real provider connected (carnet#1341). Unlike `isServerStepComplete`,
 * a `skipped` row does not count — continuing without connecting stays
 * session-only, so the connect step comes back at the next login.
 */
export function isConnectProviderRecorded(steps: PersistedOnboardingStep[] | undefined): boolean {
  return (steps ?? []).some((s) => s.step_id === 'connect_provider' && s.status === 'complete');
}

/**
 * Step status for a step outside this person's journey. Distinct from
 * `skipped`: a skipped step was offered and declined, a not-applicable one was
 * never theirs. Not a completion — `isServerStepComplete` ignores it, so the
 * step stays off the progress bar instead of reading "done".
 */
export const STEP_NOT_APPLICABLE = 'not_applicable';

/** The steps that ask about the user's own training and body. */
export const ATHLETE_STEP_IDS: readonly OnboardingStepId[] = ['about_you', 'parq'];

/**
 * Whether the server records the athlete steps as not part of this user's
 * journey — a coach who does not train, as answered on any surface. The wizard
 * marks both athlete steps; the chat intake marks `parq`, so any one suffices.
 */
export function athleteStepsWaived(steps: PersistedOnboardingStep[] | undefined): boolean {
  return (steps ?? []).some(
    (s) => ATHLETE_STEP_IDS.includes(s.step_id as OnboardingStepId) && s.status === STEP_NOT_APPLICABLE,
  );
}

/**
 * Display status for a step in the progress indicator: behind the user it is
 * `done`, or `skipped` when it was put off or passed without being finished.
 */
export type OnboardingStepStatus = 'done' | 'skipped' | 'current' | 'upcoming';

/** A step plus its progress-indicator display status. */
export interface OnboardingProgressItem {
  id: OnboardingStepId;
  /** Corpus key of the step's label; the indicator resolves it with t(). */
  labelKey: string;
  status: OnboardingStepStatus;
}

/** The journey the role question puts a user on. */
type OnboardingPath = 'athlete' | 'coach' | 'coach_and_athlete';

/** One segment of a journey's progress bar. */
interface ProgressSegment {
  id: OnboardingStepId;
}

const ATHLETE_SEGMENTS: readonly ProgressSegment[] = [
  { id: 'profile_type' },
  { id: 'about_you' },
  { id: 'parq' },
  { id: 'check_in' },
  { id: 'connect_provider' },
  { id: 'coach_proposal' },
];

/** The coach path's own segments: the seat, the group, its invite. */
const COACH_SEGMENTS: readonly ProgressSegment[] = [
  { id: 'coach_seat' },
  { id: 'coach_group' },
  { id: 'coach_invite' },
];

/**
 * Each journey's progress bar, segment by segment, in canonical order.
 *
 * Decided by the role alone — never by what the user has done, skipped or
 * been offered — so the total stays put from the role answer to the last
 * screen. The bar used to count only the steps still applicable, so a skipped
 * Connect left it and the late messaging steps joined it: 6→7→8 for an
 * athlete, 7→4→3 for a coach. A step that is no segment (the one-time consent
 * question) is never counted.
 *
 * A coach who does not train never meets Connect: their bar is Profil, then
 * the coach path (seat, group, invite). A coach who trains walks the athlete
 * journey first, then the same coach path.
 */
const PATH_SEGMENTS: Record<OnboardingPath, readonly ProgressSegment[]> = {
  athlete: ATHLETE_SEGMENTS,
  coach: [{ id: 'profile_type' }, ...COACH_SEGMENTS],
  coach_and_athlete: [...ATHLETE_SEGMENTS, ...COACH_SEGMENTS],
};

/**
 * The journey the role answer put the user on: a coach who does not train has
 * the athlete steps waived, one who trains coaches others. Until the question
 * is answered neither is recorded, so the bar shows the athlete journey — the
 * default answer — and settles on the chosen one from the next screen on.
 */
function onboardingPath(
  c: Pick<OnboardingContext, 'athleteStepsWaived' | 'coachesOthers'>,
): OnboardingPath {
  if (c.athleteStepsWaived) return 'coach';
  return c.coachesOthers ? 'coach_and_athlete' : 'athlete';
}

/**
 * This user's progress bar: their journey's segments (`PATH_SEGMENTS`), each
 * with its display status.
 *
 * Status is *position-based*: the first segment still to do is `current`,
 * those after it `upcoming`, those before it behind the user. A segment ahead
 * never consults its `isComplete` — an auto-satisfied step would otherwise
 * read done while the user is still on step one. One behind reads `done`, or
 * `skipped` when the user put it off (`skippedSteps`) or moved past it without
 * finishing it: a Connect continued without, a question added after they had
 * left its phase. A skipped step keeps its segment, so skipping, like
 * finishing, never changes the count.
 */
export function onboardingProgress(ctx: OnboardingContext): OnboardingProgressItem[] {
  const segments = PATH_SEGMENTS[onboardingPath(ctx)].flatMap((segment) => {
    const step = ONBOARDING_STEPS.find((s) => s.id === segment.id);
    return step ? [{ step, labelKey: step.labelKey }] : [];
  });
  // A step that is no segment is not a position: while one is asked, the
  // first segment still to do is current — or none, and every segment is behind.
  const currentId = firstPendingStep(
    segments.map((s) => s.step),
    ctx,
  )?.id;
  const currentIndex = currentId
    ? segments.findIndex((s) => s.step.id === currentId)
    : segments.length; // onboarding finished — everything is behind us
  return segments.map(({ step, labelKey }, i) => ({
    id: step.id,
    labelKey,
    status: i === currentIndex ? 'current' : i > currentIndex ? 'upcoming' : passedStatus(step, ctx),
  }));
}

/** How a segment behind the user reads: finished, or put off / passed unfinished. */
function passedStatus(step: OnboardingStepDef, ctx: OnboardingContext): 'done' | 'skipped' {
  return ctx.skippedSteps.includes(step.id) || !step.isComplete(ctx) ? 'skipped' : 'done';
}

/** The registry's steps the server records as put off (`skipped`): `OnboardingContext.skippedSteps`. */
export function skippedStepIds(
  steps: readonly PersistedOnboardingStep[] | undefined,
): OnboardingStepId[] {
  return ONBOARDING_STEPS.filter((def) =>
    (steps ?? []).some((s) => s.step_id === def.id && s.status === 'skipped'),
  ).map((def) => def.id);
}

/**
 * `steps` with `stepId` recorded as `status`, in place of any earlier row for
 * it. A surface writes this into its cached onboarding status as it persists
 * a step, so what reads that status — the bar's skipped segments — has the
 * answer at once instead of at the next status read.
 */
export function withStepRecorded(
  steps: readonly PersistedOnboardingStep[] | undefined,
  stepId: string,
  status: 'complete' | 'skipped' | 'not_applicable',
): PersistedOnboardingStep[] {
  return [...(steps ?? []).filter((s) => s.step_id !== stepId), { step_id: stepId, status }];
}


/**
 * The one-tap sport choices offered during onboarding.
 *
 * These strings are the VALUE stored on the athlete's profile and read back by
 * the agent, so they are English on the wire in every locale. What the chip
 * SHOWS is `SPORT_LABEL_KEY[value]`, resolved from the translation corpus.
 *
 * Both apps had their own copy of this array with the label and the value as
 * one string, which is precisely why neither could translate the chips without
 * changing what gets saved.
 */
export const ONBOARDING_SPORTS = [
  'Running',
  'Cycling',
  'Swimming',
  'Triathlon',
  'Strength',
  'Hiking',
] as const;

export type OnboardingSport = (typeof ONBOARDING_SPORTS)[number];

/** Corpus key for each sport's visible label. */
export const SPORT_LABEL_KEY: Record<OnboardingSport, string> = {
  Running: 'app.sportRunning',
  Cycling: 'app.sportCycling',
  Swimming: 'app.sportSwimming',
  Triathlon: 'app.sportTriathlon',
  Strength: 'app.sportStrength',
  Hiking: 'app.sportHiking',
};

/**
 * Whether a stored sport is one of the one-tap choices.
 *
 * The profile field is free text — the onboarding screen offers these six and
 * lets an athlete type anything else — so a read-back has to check before
 * reaching for a label key, or an unlisted sport renders as a missing-key
 * string instead of what the athlete actually typed.
 */
export function isOnboardingSport(value: string): value is OnboardingSport {
  return (ONBOARDING_SPORTS as readonly string[]).includes(value);
}

/**
 * Catalogue key per canonical activity sport, and the provider aliases that
 * fold onto those canonical names. The table lives in `activity-sports.json`
 * so the server reads the same vocabulary (`pierre_services::activity_sports`)
 * when it names a sport in a messaging reply.
 */
export const ACTIVITY_SPORT_LABEL_KEY: Record<string, string> = activitySports.labelKeys;

const ACTIVITY_SPORT_ALIASES: Record<string, string> = activitySports.aliases;

/**
 * The label key for a sport as the wire spells it, or `null` for a sport the
 * vocabulary does not know — the caller then shows the wire spelling itself.
 *
 * The proposal profile has carried title-cased display strings
 * (`"Trail Running"`, `"Kayaking V2"`) as well as serde names, so the lookup
 * folds case and spacing and drops a provider's version suffix before it
 * matches; the same call keeps working once the wire carries the codes.
 */
export function activitySportLabelKey(sport: string): string | null {
  return ACTIVITY_SPORT_LABEL_KEY[canonicalActivitySport(sport)] ?? null;
}

/** A sport as the wire spells it, folded onto the vocabulary's canonical name. */
function canonicalActivitySport(sport: string): string {
  const folded = sport
    .trim()
    .toLowerCase()
    .replace(/[\s-]+/g, '_')
    .replace(/_v\d+$/, '');
  return ACTIVITY_SPORT_ALIASES[folded] ?? folded;
}

/**
 * The sports the agent's route search covers — the `sport_type` values the
 * `discover_routes` tool accepts. Anything else (a swim, a strength session,
 * a ride on the trainer) has no route to look for.
 */
const ROUTE_SPORTS: ReadonlySet<string> = new Set([
  'run',
  'trail_running',
  'ride',
  'mountain_bike',
  'gravel_ride',
  'ebike_ride',
  'hike',
  'walk',
  'cross_country_skiing',
  'alpine_skiing',
  'backcountry_skiing',
  'snowshoe',
]);

/**
 * Whether a session of this sport happens on a route the agent can look up,
 * so a surface offers "find a route" only where there is one to find. A sport
 * the vocabulary does not know is answered no: the offer is withheld rather
 * than made for a session it may not fit.
 */
export function sportHasRoutes(sport: string): boolean {
  return ROUTE_SPORTS.has(canonicalActivitySport(sport));
}

/**
 * The sports covered on foot — the ones whose recorded map carries start,
 * finish and distance markers. A ride, a swim or a ski covers distance too
 * fast, or along no line worth counting, for a mark every kilometre to read as
 * anything but clutter, so only these four get them.
 */
const ON_FOOT_SPORTS: ReadonlySet<string> = new Set(['run', 'trail_running', 'walk', 'hike']);

/**
 * Whether a recorded session of this sport was covered on foot. A sport the
 * vocabulary does not know is answered no, like {@link sportHasRoutes}.
 */
export function sportIsOnFoot(sport: string): boolean {
  return ON_FOOT_SPORTS.has(canonicalActivitySport(sport));
}

/**
 * The sports an activity is run in shoes for: the on-foot sports, and a
 * treadmill run, which the server counts on a `run` pair (`sport_matches_family`
 * in pierre-core). The athlete is offered a pair of their shoes for these
 * activities only.
 */
const SHOE_SPORTS: ReadonlySet<string> = new Set([...ON_FOOT_SPORTS, 'virtual_run']);

/**
 * Whether an activity of this sport is one the gear ledger counts a pair of
 * shoes for. A sport the vocabulary does not know is answered no, like
 * {@link sportIsOnFoot}.
 */
export function sportTakesShoes(sport: string): boolean {
  return SHOE_SPORTS.has(canonicalActivitySport(sport));
}

/**
 * How long, in days, the athlete invite a coach leaves onboarding with stays
 * valid. Longer than the 7 days a chat-issued invite gets: a coach sets up
 * during onboarding, often before the athlete is ready to sign up, and a week
 * is easily missed.
 */
export const ONBOARDING_INVITE_DAYS = 30;
