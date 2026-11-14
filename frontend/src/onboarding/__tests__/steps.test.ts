// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the onboarding step registry — decision engine + progress status computation
// ABOUTME: Pins currentOnboardingStep as a faithful translation of the former App.tsx gate chain (all scenarios)

import { describe, it, expect } from 'vitest';
import {
  athleteStepsWaived,
  currentOnboardingStep,
  healthCheckOutstanding,
  isConnectProviderRecorded,
  isConnectProviderSkipped,
  isServerStepComplete,
  onboardingProgress,
  ONBOARDING_STEPS,
  skippedStepIds,
  withStepRecorded,
  type OnboardingContext,
  type OnboardingProgressItem,
  type OnboardingStepId,
} from '../steps';

/** A fully-onboarded, returning-user baseline; each test overrides only what it exercises. */
function ctx(overrides: Partial<OnboardingContext> = {}): OnboardingContext {
  return {
    onboardingActive: true,
    needsProviderConnection: false,
    // Never connected unless a test says so: the pre-connect phase is open
    // whenever `needsProviderConnection` is true.
    connectProviderRecorded: false,
    skippedProvider: false,
    skippedSteps: [],
    justOnboarded: false,
    // Continue on the connect step already pressed, so a just-connected test
    // exercises the steps after it; the connect-another tests opt out.
    connectProviderDone: true,
    profileTypeChosen: true,
    coachProposalDone: true,
    // The PAR-Q defaults to done in this baseline, so a test exercising a
    // later step isn't intercepted by it.
    parqDone: true,
    athleteStepsWaived: false,
    // An athlete unless a test says otherwise: the group step is a coach's.
    coachesOthers: false,
    coachGroupDone: false,
    coachSeatDone: false,
    coachGroupCreated: false,
    // Answered unless a test says otherwise, so the consent question does not
    // intercept the journey steps the other tests exercise.
    analyticsConsentAnswered: true,
    ...overrides,
  };
}

describe('currentOnboardingStep — equivalent to the former App.tsx gate chain', () => {
  it('fresh active user (needs a provider, nothing chosen) → profile_type', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
      )?.id,
    ).toBe('profile_type');
  });

  it('profile chosen but still needs a provider → connect_provider', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: true, profileTypeChosen: true, coachProposalDone: false }),
      )?.id,
    ).toBe('connect_provider');
  });

  it('the PAR-Q is never asked before Tes données: profile chosen, PAR-Q open → connect_provider', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: true,
          profileTypeChosen: true,
          parqDone: false,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('connect_provider');
  });

  it('connected, Continue pressed, PAR-Q open → parq, the third screen, before the team', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: false,
          connectProviderRecorded: true,
          parqDone: false,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('parq');
  });

  it('continued without connecting, PAR-Q open → parq, then the team', () => {
    const skipped = ctx({
      needsProviderConnection: true,
      skippedProvider: true,
      parqDone: false,
      coachProposalDone: false,
    });
    expect(currentOnboardingStep(skipped)?.id).toBe('parq');
    expect(currentOnboardingStep({ ...skipped, parqDone: true })?.id).toBe('coach_proposal');
  });

  it('an athlete past the team step is never sent back for the PAR-Q: Home asks it', () => {
    expect(currentOnboardingStep(ctx({ parqDone: false }))).toBeNull();
  });

  it('the PAR-Q waits for the status: nothing is asked while it is loading', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: undefined, skippedProvider: true, parqDone: false, coachProposalDone: false }),
      )?.id,
    ).not.toBe('parq');
  });

  it('just connected this session (needs=false, justOnboarded) → coach_proposal', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
      )?.id,
    ).toBe('coach_proposal');
  });

  it('just connected, Continue not pressed, PAR-Q open → still connect_provider, the PAR-Q after it', () => {
    const waiting = ctx({
      needsProviderConnection: false,
      justOnboarded: true,
      connectProviderDone: false,
      parqDone: false,
      coachProposalDone: false,
    });
    expect(currentOnboardingStep(waiting)?.id).toBe('connect_provider');
    expect(currentOnboardingStep({ ...waiting, connectProviderDone: true })?.id).toBe('parq');
  });

  it('just connected, Continue not pressed yet → stays on connect_provider to add another', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: false,
          justOnboarded: true,
          connectProviderDone: false,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('connect_provider');
  });

  it('a returning connected user is never shown connect_provider, Continue or not', () => {
    expect(
      currentOnboardingStep(ctx({ needsProviderConnection: false, connectProviderDone: false })),
    ).toBeNull();
  });

  // carnet#1341: an onboarded athlete who disconnects their last provider
  // turns `needsProviderConnection` back to true. The server's connect-step
  // record keeps them out of the wizard; a user who never connected has no
  // record and still meets the connect step.
  it('an onboarded athlete who disconnected every provider stays out of the wizard', () => {
    const disconnected = ctx({ needsProviderConnection: true, connectProviderRecorded: true });
    expect(currentOnboardingStep(disconnected)).toBeNull();
    // Not even a PAR-Q they never answered: Home asks it.
    expect(currentOnboardingStep({ ...disconnected, parqDone: false })).toBeNull();
  });

  it('an athlete who disconnects before the agent step still meets it, not the connect step', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: true,
          connectProviderRecorded: true,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('coach_proposal');
  });

  it('a user who never connected still meets the connect step at each login', () => {
    expect(
      currentOnboardingStep(ctx({ needsProviderConnection: true, connectProviderRecorded: false }))
        ?.id,
    ).toBe('connect_provider');
  });

  it('the first connection still keeps the connect step up until Continue', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: false,
          connectProviderRecorded: true,
          justOnboarded: true,
          connectProviderDone: false,
        }),
      )?.id,
    ).toBe('connect_provider');
  });

  it('connect_provider reads incomplete while waiting on Continue, complete after', () => {
    const step = ONBOARDING_STEPS.find((s) => s.id === 'connect_provider');
    const waiting = ctx({ needsProviderConnection: false, justOnboarded: true, connectProviderDone: false });
    expect(step?.isComplete(waiting)).toBe(false);
    expect(step?.isComplete({ ...waiting, connectProviderDone: true })).toBe(true);
    expect(step?.isComplete(ctx({ needsProviderConnection: true }))).toBe(false);
  });

  it('returning already-onboarded user → null (dashboard)', () => {
    expect(currentOnboardingStep(ctx())).toBeNull();
  });

  it('returning user with cleared localStorage never re-sees profile_type (needs=false)', () => {
    // Regression guard: profile_type is gated on needs===true, so a
    // long-connected user with no local flags must NOT be pulled back into it.
    // Their agent step is done by the server record, which survives the clear.
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, profileTypeChosen: false, coachProposalDone: true }),
      ),
    ).toBeNull();
  });

  it('"continue without connecting" (skipped) → coach_proposal, then the dashboard', () => {
    // The team step offers the agents that need no provider, so an athlete
    // who skips connecting still reaches Home with a team (carnet#1095).
    const skipped = ctx({
      needsProviderConnection: true,
      skippedProvider: true,
      profileTypeChosen: false,
      coachProposalDone: false,
    });
    expect(currentOnboardingStep(skipped)?.id).toBe('coach_proposal');
    expect(currentOnboardingStep({ ...skipped, coachProposalDone: true })).toBeNull();
  });

  it('admin / inactive user (onboardingActive=false, needs=undefined) → null', () => {
    expect(
      currentOnboardingStep(
        ctx({ onboardingActive: false, needsProviderConnection: undefined }),
      ),
    ).toBeNull();
  });

  it('fresh tab, provider already connected, coach step unrecorded → coach_proposal (carnet#1042)', () => {
    // The OAuth return landed in a tab that never saw needs true→false, so
    // justOnboarded is false; the server still says a provider is connected
    // and the step is not done, so the proposal is shown.
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, justOnboarded: false, coachProposalDone: false }),
      )?.id,
    ).toBe('coach_proposal');
  });

  it('coach step recorded done or declined → not shown, with or without the flip', () => {
    for (const justOnboarded of [false, true]) {
      expect(
        currentOnboardingStep(
          ctx({ needsProviderConnection: false, justOnboarded, coachProposalDone: true }),
        ),
      ).toBeNull();
    }
  });

  it('coach_proposal waits for the server to report a connected provider', () => {
    // Status still loading (undefined) or no provider yet: the agent step is not current.
    expect(
      currentOnboardingStep(ctx({ needsProviderConnection: undefined, coachProposalDone: false })),
    ).toBeNull();
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: true, coachProposalDone: false }),
      )?.id,
    ).toBe('connect_provider');
  });
});

/**
 * Messaging channels configured for the tenant. Spelled loosely on purpose:
 * the messaging steps are leaving the wizard, and the tests below must hold —
 * and compile — once their context fields go with them.
 */
const WITH_CHANNELS = { messagingAvailableCount: 2 } as Record<string, unknown> as Partial<OnboardingContext>;

describe('onboardingProgress — the journey the role decides', () => {
  it('draws the athlete journey as six segments named for their screens', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
    );
    expect(items.map((i) => i.labelKey)).toEqual([
      'onboarding.stepProfile',
      'onboarding.stepConnect',
      'onboarding.stepHealthCheck',
      'onboarding.stepAgent',
    ]);
  });

  it('draws a coach who does not train four segments: profile, coach seat, group, invite', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, athleteStepsWaived: true, coachesOthers: true }),
    );
    expect(items.map((i) => i.labelKey)).toEqual([
      'onboarding.stepProfile',
      'humanCoach.stepSeat',
      'onboarding.stepGroup',
      'onboarding.stepInvite',
    ]);
  });

  it('draws a coach who trains the athlete journey, then the coach path after it', () => {
    const items = onboardingProgress(ctx({ needsProviderConnection: true, coachesOthers: true }));
    expect(items.map((i) => i.labelKey)).toEqual([
      'onboarding.stepProfile',
      'onboarding.stepConnect',
      'onboarding.stepHealthCheck',
      'onboarding.stepAgent',
      'humanCoach.stepSeat',
      'onboarding.stepGroup',
      'onboarding.stepInvite',
    ]);
  });

  // Found by the 2026-10-23 onboarding e2e on dev: the bar read "5 of 5" on the
  // connect step and "6 of 7" on the next one. The total is the journey's from
  // the start, so it never grows as the athlete advances.
  it('keeps the same total from the first step to the last', () => {
    const steps = { ...WITH_CHANNELS, coachProposalDone: false };
    const first = onboardingProgress(
      ctx({
        ...steps,
        needsProviderConnection: true,
        profileTypeChosen: false,
        parqDone: false,
      }),
    );
    const onConnect = onboardingProgress(ctx({ ...steps, needsProviderConnection: true, parqDone: false }));
    const onHealth = onboardingProgress(ctx({ ...steps, needsProviderConnection: false, parqDone: false }));
    const onTeam = onboardingProgress(ctx({ ...steps, needsProviderConnection: false }));
    const afterTeam = onboardingProgress(
      ctx({ ...steps, needsProviderConnection: false, coachProposalDone: true }),
    );
    const totals = [first, onConnect, onHealth, onTeam, afterTeam].map((items) => items.length);
    expect(totals, 'the journey length must not change as the athlete advances').toEqual([4, 4, 4, 4, 4]);
  });

  it('keeps canonical order — the journey is a subset, never a reshuffle', () => {
    for (const role of [{}, { coachesOthers: true }, { athleteStepsWaived: true, coachesOthers: true }]) {
      const items = onboardingProgress(ctx({ ...role, needsProviderConnection: true }));
      const canonical = ONBOARDING_STEPS.map((s) => s.id);
      const positions = items.map((i) => canonical.indexOf(i.id));
      expect(positions).toEqual([...positions].sort((a, b) => a - b));
      expect(positions.every((p) => p >= 0)).toBe(true);
    }
  });

  it('never counts a messaging step, whatever channels the tenant has', () => {
    for (const channels of [{}, WITH_CHANNELS]) {
      const items = onboardingProgress(
        ctx({ ...channels, needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
      );
      expect(items).toHaveLength(4);
    }
  });

  it('before the role is answered, shows the athlete journey — the default answer', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
    );
    expect(items).toHaveLength(4);
    expect(items[0]).toMatchObject({ id: 'profile_type', status: 'current' });
  });

  /**
   * The contract `onboardingProgress` actually promises is *positional*: the
   * current step is `current`, everything before it is `done`, everything after
   * is `upcoming`. Asserting that invariant — rather than a snapshot of the
   * pipeline's length — is what keeps these tests meaningful when a step is
   * added. The label tests above still pin each journey, so a change to one
   * remains deliberate rather than silent.
   */
  function expectPositional(items: ReturnType<typeof onboardingProgress>, currentId: string) {
    const currentIndex = items.findIndex((i) => i.id === currentId);
    expect(currentIndex, `${currentId} should be in the pipeline`).toBeGreaterThanOrEqual(0);
    expect(items.map((i) => i.status)).toEqual(
      items.map((_, i) => (i === currentIndex ? 'current' : i < currentIndex ? 'done' : 'upcoming')),
    );
  }

  it('on profile_type: it is current and nothing before it is done', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: true,
        profileTypeChosen: false,
        parqDone: false,
        coachProposalDone: false,
      }),
    );
    expectPositional(items, 'profile_type');
    expect(items[0].status).toBe('current');
  });

  it('on connect_provider: profile_type behind it is done', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: true, parqDone: false, coachProposalDone: false }),
    );
    expectPositional(items, 'connect_provider');
  });

  it('on parq: Profil and Tes données behind it are done', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        connectProviderRecorded: true,
        profileTypeChosen: true,
        parqDone: false,
        coachProposalDone: false,
      }),
    );
    expectPositional(items, 'parq');
  });

  it('an invited athlete starts on Tes données with Profil done: the role is recorded, not special-cased', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: true, parqDone: false, coachProposalDone: false }),
    );
    expect(items).toHaveLength(4);
    expect(items[0]).toMatchObject({ id: 'profile_type', status: 'done' });
    expect(items[1]).toMatchObject({ id: 'connect_provider', status: 'current' });
  });

  it('on coach_proposal: the provider gate is behind it', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
    );
    expectPositional(items, 'coach_proposal');
  });

  it('past the last segment, every segment is behind and none is current', () => {
    // Whatever is asked after the journey's last segment — a chat app, for as
    // long as the wizard offers one — is no segment of its own.
    const items = onboardingProgress(
      ctx({ ...WITH_CHANNELS, needsProviderConnection: false, justOnboarded: true, coachProposalDone: true }),
    );
    expect(items.map((i) => i.status)).toEqual(items.map(() => 'done'));
  });
});

/** The answer a walk gives each step: finish it, or put it off. */
type Answer = 'complete' | 'skip';
type Role = 'athlete' | 'coach' | 'coach_and_athlete';

/** One screen of a walk: the step on it, and the bar drawn over it (none for the consent question). */
interface Screen {
  step: OnboardingStepId;
  bar: OnboardingProgressItem[] | null;
}

/** A brand-new account: nothing answered, no provider, the usage question not yet asked. */
function freshAccount(overrides: Partial<OnboardingContext> = {}): OnboardingContext {
  return ctx({
    needsProviderConnection: true,
    connectProviderDone: false,
    profileTypeChosen: false,
    parqDone: false,
    coachProposalDone: false,
    analyticsConsentAnswered: false,
    ...overrides,
  });
}

/** The context once `step` is answered, the way the surfaces write it. */
function advance(c: OnboardingContext, step: OnboardingStepId, role: Role, answer: Answer): OnboardingContext {
  const putOff = (next: OnboardingContext) =>
    answer === 'skip' ? { ...next, skippedSteps: [...next.skippedSteps, step] } : next;
  switch (step) {
    case 'analytics_consent':
      return { ...c, analyticsConsentAnswered: true };
    case 'profile_type':
      return {
        ...c,
        profileTypeChosen: true,
        coachesOthers: role !== 'athlete',
        athleteStepsWaived: role === 'coach',
      };
    case 'parq':
      return putOff({ ...c, parqDone: true });
    case 'connect_provider':
      // The skip is the connect step's skipped row, which the bar reads too.
      if (answer === 'skip') return putOff({ ...c, skippedProvider: true });
      // The OAuth return keeps the step up for a second service; Continue leaves it.
      return c.needsProviderConnection
        ? { ...c, needsProviderConnection: false, connectProviderRecorded: true, justOnboarded: true }
        : { ...c, connectProviderDone: true };
    case 'coach_proposal':
      return putOff({ ...c, coachProposalDone: true, justOnboarded: false });
    case 'coach_seat':
      // Continue and « Passer pour l'instant » set the same device flag and
      // write no step row, so a seat put off is not told from one settled.
      return { ...c, coachSeatDone: true };
    case 'coach_group':
      // Making the group moves the coach on to its invite; putting it off
      // records the group step, which leaves nothing to invite to.
      return answer === 'skip' ? putOff({ ...c, coachGroupDone: true }) : { ...c, coachGroupCreated: true };
    case 'coach_invite':
      // The invite has one way on, « Aller au groupe », which records the group step.
      return { ...c, coachGroupDone: true };
    default: {
      const unknown: never = step;
      throw new Error(`advance() does not know the step ${String(unknown)}`);
    }
  }
}

/** Every screen the wizard shows on one journey, from sign-up to Home. */
function walk(
  role: Role,
  answer: (step: OnboardingStepId) => Answer,
  start: OnboardingContext = freshAccount(),
): Screen[] {
  const screens: Screen[] = [];
  let c = start;
  for (let guard = 0; guard < 30; guard++) {
    const step = currentOnboardingStep(c);
    if (!step) return screens;
    screens.push({ step: step.id, bar: step.hiddenFromProgress ? null : onboardingProgress(c) });
    c = advance(c, step.id, role, answer(step.id));
  }
  throw new Error(`the ${role} walk never left the wizard`);
}

/** The bar on each screen after the role is answered — the screens whose total the role decides. */
function barsAfterRole(screens: Screen[]): { step: OnboardingStepId; bar: OnboardingProgressItem[] }[] {
  const role = screens.findIndex((s) => s.step === 'profile_type');
  return screens
    .slice(role + 1)
    .flatMap((s) => (s.bar ? [{ step: s.step, bar: s.bar }] : []));
}

const ALWAYS_FINISH = () => 'complete' as const;
const ALWAYS_SKIP = () => 'skip' as const;

describe('onboardingProgress — one total on every screen of a journey', () => {
  const TOTALS: Record<Role, number> = { athlete: 4, coach: 4, coach_and_athlete: 7 };
  const ROLES: Role[] = ['athlete', 'coach', 'coach_and_athlete'];
  const ANSWERS = { finishing: ALWAYS_FINISH, skipping: ALWAYS_SKIP };

  for (const role of ROLES) {
    for (const [how, answer] of Object.entries(ANSWERS)) {
      for (const [tenant, channels] of Object.entries({ 'no chat app': {}, 'chat apps': WITH_CHANNELS })) {
        it(`${role}, ${how} every step, ${tenant}: the same total on every screen`, () => {
          const screens = walk(role, answer, freshAccount(channels));
          const bars = barsAfterRole(screens);
          expect(bars.length, 'the walk reached screens after the role question').toBeGreaterThan(0);
          expect(bars.map((b) => b.bar.length)).toEqual(bars.map(() => TOTALS[role]));
        });
      }
    }
  }

  it('the role screen draws the athlete journey; the athlete keeps it to the end', () => {
    for (const answer of [ALWAYS_FINISH, ALWAYS_SKIP]) {
      const totals = walk('athlete', answer).flatMap((s) => (s.bar ? [s.bar.length] : []));
      expect(totals).toEqual(totals.map(() => 4));
    }
  });

  it('an athlete reaches Home in four screens after the consent question: Profil, Tes données, Santé, Ton équipe', () => {
    for (const answer of [ALWAYS_FINISH, ALWAYS_SKIP]) {
      const steps = walk('athlete', answer).map((s) => s.step);
      // Connecting keeps Tes données up for a second service until Continue:
      // one screen, drawn twice.
      expect([...new Set(steps)]).toEqual(['analytics_consent', 'profile_type', 'connect_provider', 'parq', 'coach_proposal']);
    }
  });

  it('an invited athlete, the role recorded for them, taps through three screens with Profil done', () => {
    const screens = walk('athlete', ALWAYS_FINISH, freshAccount({ profileTypeChosen: true, analyticsConsentAnswered: true }));
    expect([...new Set(screens.map((s) => s.step))]).toEqual(['connect_provider', 'parq', 'coach_proposal']);
    for (const { bar } of screens) {
      expect(bar?.[0]).toMatchObject({ id: 'profile_type', status: 'done' });
      expect(bar).toHaveLength(4);
    }
  });

  it('every screen that is a segment is the current one, in the order the bar draws them', () => {
    for (const role of ROLES) {
      for (const { step, bar } of barsAfterRole(walk(role, ALWAYS_FINISH))) {
        const current = bar.filter((s) => s.status === 'current').map((s) => s.id);
        expect(current, `${role} on ${step}`).toEqual([step]);
      }
    }
  });

  it('an athlete who skips everything sees each skipped step stay on the bar as skipped', () => {
    const team = walk('athlete', ALWAYS_SKIP).find((s) => s.step === 'coach_proposal');
    expect(team?.bar?.map((s) => [s.id, s.status])).toEqual([
      ['profile_type', 'done'],
      ['connect_provider', 'skipped'],
      ['parq', 'skipped'],
      ['coach_proposal', 'current'],
    ]);
  });

  it('an athlete who finishes everything sees each step done', () => {
    const team = walk('athlete', ALWAYS_FINISH).find((s) => s.step === 'coach_proposal');
    expect(team?.bar?.map((s) => s.status)).toEqual(['done', 'done', 'done', 'current']);
  });

  it('a coach who skips the seat keeps it on the bar, behind them, on the group screen', () => {
    // A seat put off is a device flag that is not told from one settled, and
    // no step row: it reads done, and keeps its segment.
    const group = walk('coach', ALWAYS_SKIP).find((s) => s.step === 'coach_group');
    expect(group?.bar?.map((s) => [s.labelKey, s.status])).toEqual([
      ['onboarding.stepProfile', 'done'],
      ['humanCoach.stepSeat', 'done'],
      ['onboarding.stepGroup', 'current'],
      ['onboarding.stepInvite', 'upcoming'],
    ]);
  });

  it('a coach who connects keeps the seat done on the group screen, and the group on the invite', () => {
    const screens = walk('coach', ALWAYS_FINISH);
    expect(screens.find((s) => s.step === 'coach_group')?.bar?.map((s) => s.status)).toEqual([
      'done',
      'done',
      'current',
      'upcoming',
    ]);
    expect(screens.find((s) => s.step === 'coach_invite')?.bar?.map((s) => s.status)).toEqual([
      'done',
      'done',
      'done',
      'current',
    ]);
  });

  it('skipping only the PAR-Q leaves the other steps done', () => {
    const group = walk('coach_and_athlete', (step) => (step === 'parq' ? 'skip' : 'complete')).find(
      (s) => s.step === 'coach_group',
    );
    expect(group?.bar?.map((s) => s.status)).toEqual([
      'done',
      'done',
      'skipped',
      'done',
      'done',
      'current',
      'upcoming',
    ]);
  });

  it('a coach who trains and skips Connect keeps seven segments to the end of the wizard', () => {
    // Skipping leads on to the coach path rather than past it, so the wizard
    // ends on the invite with Connect behind as skipped — the total does not move.
    const screens = walk('coach_and_athlete', (step) => (step === 'connect_provider' ? 'skip' : 'complete'));
    const last = barsAfterRole(screens).at(-1);
    expect(last?.step).toBe('coach_invite');
    expect(last?.bar.map((s) => s.status)).toEqual([
      'done',
      'skipped',
      'done',
      'done',
      'done',
      'done',
      'current',
    ]);
  });

  it('a skip recorded on another device reads skipped here too', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        coachProposalDone: false,
        skippedSteps: skippedStepIds([{ step_id: 'parq', status: 'skipped' }]),
      }),
    );
    expect(items.find((s) => s.id === 'parq')?.status).toBe('skipped');
  });
});

describe('skippedStepIds — the server rows the bar reads a skip from', () => {
  it('lists the registry steps recorded skipped, in canonical order', () => {
    expect(
      skippedStepIds([
        { step_id: 'parq', status: 'skipped' },
        { step_id: 'connect_provider', status: 'skipped' },
        { step_id: 'coach_proposal', status: 'complete' },
        { step_id: 'parq', status: 'not_applicable' },
      ]),
    ).toEqual(['connect_provider', 'parq']);
  });

  it('never lists a step Home asks: about_you and check_in are no segment', () => {
    expect(
      skippedStepIds([
        { step_id: 'about_you', status: 'skipped' },
        { step_id: 'check_in', status: 'skipped' },
      ]),
    ).toEqual([]);
  });

  it('ignores a step the registry does not know, and no steps at all', () => {
    expect(skippedStepIds([{ step_id: 'a_future_step', status: 'skipped' }])).toEqual([]);
    expect(skippedStepIds(undefined)).toEqual([]);
  });
});

describe('withStepRecorded — the row a surface caches as it writes a step', () => {
  it('adds the row, replacing an earlier one for the same step', () => {
    expect(
      withStepRecorded(
        [
          { step_id: 'about_you', status: 'complete' },
          { step_id: 'parq', status: 'complete' },
        ],
        'about_you',
        'skipped',
      ),
    ).toEqual([
      { step_id: 'parq', status: 'complete' },
      { step_id: 'about_you', status: 'skipped' },
    ]);
  });

  it('starts from no rows', () => {
    expect(withStepRecorded(undefined, 'check_in', 'complete')).toEqual([
      { step_id: 'check_in', status: 'complete' },
    ]);
  });
});

describe('isServerStepComplete — durable cross-device step state', () => {
  it('treats a server "complete" record as done', () => {
    const steps = [{ step_id: 'profile_type', status: 'complete' }];
    expect(isServerStepComplete(steps, 'profile_type')).toBe(true);
  });

  it('treats a server "skipped" record as done', () => {
    const steps = [{ step_id: 'coach_proposal', status: 'skipped' }];
    expect(isServerStepComplete(steps, 'coach_proposal')).toBe(true);
  });

  it('is false for an unrecorded step, and safe on undefined steps', () => {
    const steps = [{ step_id: 'profile_type', status: 'complete' }];
    expect(isServerStepComplete(steps, 'coach_proposal')).toBe(false);
    expect(isServerStepComplete(undefined, 'profile_type')).toBe(false);
  });

  it('ignores unknown statuses (e.g. a future "pending")', () => {
    const steps = [{ step_id: 'profile_type', status: 'pending' }];
    expect(isServerStepComplete(steps, 'profile_type')).toBe(false);
  });
});

// carnet#1341: only the server's `complete` record closes the pre-connect
// phase. A `skipped` row must not, or a user who continued without connecting
// would never meet the connect step again.
describe('isConnectProviderRecorded — the connect step is past for good', () => {
  it('is true for a server "complete" record', () => {
    expect(isConnectProviderRecorded([{ step_id: 'connect_provider', status: 'complete' }])).toBe(true);
  });

  it('is false for a "skipped" record, another step, or no steps', () => {
    expect(isConnectProviderRecorded([{ step_id: 'connect_provider', status: 'skipped' }])).toBe(false);
    expect(isConnectProviderRecorded([{ step_id: 'coach_proposal', status: 'complete' }])).toBe(false);
    expect(isConnectProviderRecorded(undefined)).toBe(false);
  });
});

describe('isConnectProviderSkipped — « Continuer sans connecter » is saved', () => {
  it('is true for a server "skipped" connect row, and nothing else', () => {
    expect(isConnectProviderSkipped([{ step_id: 'connect_provider', status: 'skipped' }])).toBe(true);
    expect(isConnectProviderSkipped([{ step_id: 'connect_provider', status: 'complete' }])).toBe(false);
    expect(isConnectProviderSkipped([{ step_id: 'parq', status: 'skipped' }])).toBe(false);
    expect(isConnectProviderSkipped(undefined)).toBe(false);
  });

  it('keeps Connect out of the wizard at the next sign-in: the PAR-Q, then the team', () => {
    // A new session: the tab-local tap is gone, the server row is not.
    const nextSignIn = ctx({
      needsProviderConnection: true,
      skippedProvider: isConnectProviderSkipped([{ step_id: 'connect_provider', status: 'skipped' }]),
      parqDone: false,
      coachProposalDone: false,
    });
    expect(currentOnboardingStep(nextSignIn)?.id).toBe('parq');
    expect(currentOnboardingStep({ ...nextSignIn, parqDone: true, coachProposalDone: true })).toBeNull();
  });
});

describe('healthCheckOutstanding — « Construire mon plan » asks the PAR-Q first', () => {
  it('holds while the PAR-Q has no complete row: put off, or never asked', () => {
    expect(healthCheckOutstanding([{ step_id: 'parq', status: 'skipped' }])).toBe(true);
    expect(healthCheckOutstanding([])).toBe(true);
    expect(healthCheckOutstanding(undefined)).toBe(true);
  });

  it('opens once the PAR-Q is answered, and never holds a coach who does not train', () => {
    expect(healthCheckOutstanding([{ step_id: 'parq', status: 'complete' }])).toBe(false);
    expect(healthCheckOutstanding([{ step_id: 'parq', status: 'not_applicable' }])).toBe(false);
    expect(healthCheckOutstanding([{ step_id: 'about_you', status: 'not_applicable' }])).toBe(false);
  });
});

describe('coach who does not train — athlete steps leave the journey', () => {
  const coachOnly = (overrides: Partial<OnboardingContext> = {}) =>
    ctx({
      needsProviderConnection: true,
      profileTypeChosen: true,
      parqDone: false,
      coachProposalDone: false,
      athleteStepsWaived: true,
      coachesOthers: true,
      ...overrides,
    });

  it('goes straight from profile_type to the coach seat, never asking the PAR-Q or to connect', () => {
    expect(currentOnboardingStep(coachOnly())?.id).toBe('coach_seat');
  });

  it('is never screened past the gate either: no load is prescribed to them', () => {
    expect(currentOnboardingStep(coachOnly({ needsProviderConnection: false }))?.id).toBe('coach_seat');
  });

  it('meets four screens on the bar: profile, coach seat, group, invite', () => {
    expect(onboardingProgress(coachOnly()).map((s) => s.id)).toEqual([
      'profile_type',
      'coach_seat',
      'coach_group',
      'coach_invite',
    ]);
  });

  it('counts no messaging step ahead of a coach on the seat step', () => {
    expect(
      onboardingProgress(coachOnly({ coachesOthers: true, ...WITH_CHANNELS })).map((s) => s.id),
    ).toEqual(['profile_type', 'coach_seat', 'coach_group', 'coach_invite']);
  });

  it('keeps the same four segments from the first coach screen to the last', () => {
    const screens = [
      coachOnly(),
      coachOnly({ coachSeatDone: true }),
      coachOnly({ coachSeatDone: true, coachGroupCreated: true }),
    ];
    expect(screens.map((c) => currentOnboardingStep(c)?.id)).toEqual([
      'coach_seat',
      'coach_group',
      'coach_invite',
    ]);
    for (const c of screens) {
      expect(onboardingProgress(c).map((s) => s.labelKey)).toEqual([
        'onboarding.stepProfile',
        'humanCoach.stepSeat',
        'onboarding.stepGroup',
        'onboarding.stepInvite',
      ]);
    }
    expect(onboardingProgress(screens[2]).map((s) => s.status)).toEqual([
      'done',
      'done',
      'done',
      'current',
    ]);
  });

  it('a skipped seat stays on the bar, behind the coach', () => {
    const onGroup = coachOnly({ coachSeatDone: true });
    expect(onboardingProgress(onGroup).map((s) => [s.id, s.status])).toEqual([
      ['profile_type', 'done'],
      ['coach_seat', 'done'],
      ['coach_group', 'current'],
      ['coach_invite', 'upcoming'],
    ]);
  });

  it('a coach who trains still gets the athlete steps: Tes données, then the PAR-Q', () => {
    const trains = coachOnly({ athleteStepsWaived: false });
    expect(currentOnboardingStep(trains)?.id).toBe('connect_provider');
    expect(currentOnboardingStep({ ...trains, skippedProvider: true })?.id).toBe('parq');
  });

  it('is never offered an agent for their own training — the seat, then the group', () => {
    const connected = coachOnly({ needsProviderConnection: false, justOnboarded: true });
    expect(currentOnboardingStep(connected)?.id).toBe('coach_seat');
    expect(onboardingProgress(connected).map((s) => s.id)).not.toContain('coach_proposal');
    expect(onboardingProgress(connected).map((s) => s.id)).not.toContain('connect_provider');
  });

  it('keeps four segments once a coaching platform is connected, after a reload too', () => {
    for (const justOnboarded of [true, false]) {
      const connected = coachOnly({ needsProviderConnection: false, justOnboarded, connectProviderDone: false });
      expect(onboardingProgress(connected).map((s) => s.id)).toEqual([
        'profile_type',
        'coach_seat',
        'coach_group',
        'coach_invite',
      ]);
    }
  });

  it('putting the group off ends the journey: nothing is left to invite to', () => {
    expect(currentOnboardingStep(coachOnly({ coachSeatDone: true, coachGroupDone: true }))).toBeNull();
  });

  it('skipping, then finishing the group, ends the wizard: no chat-app step follows', () => {
    expect(
      currentOnboardingStep(
        coachOnly({ coachesOthers: true, skippedProvider: true, coachGroupDone: true }),
      ),
    ).toBeNull();
  });

  it('is not sent back to the seat at the next login once the group exists', () => {
    expect(currentOnboardingStep(coachOnly({ coachGroupDone: true }))).toBeNull();
  });

  it('meets no coach step while the onboarding status is loading', () => {
    expect(currentOnboardingStep(coachOnly({ needsProviderConnection: undefined }))).toBeNull();
  });

  it('a coach who trains is held at connect_provider for their own data first', () => {
    const trains = coachOnly({
      athleteStepsWaived: false,
      parqDone: true,
    });
    expect(currentOnboardingStep(trains)?.id).toBe('connect_provider');
    expect(currentOnboardingStep({ ...trains, skippedProvider: true })?.id).toBe('coach_proposal');
  });

  it('a coach who trains and skips the connect step still reaches the seat and the group', () => {
    const skipped = coachOnly({
      athleteStepsWaived: false,
      parqDone: true,
      skippedProvider: true,
      coachProposalDone: true,
    });
    expect(currentOnboardingStep(skipped)?.id).toBe('coach_seat');
    expect(currentOnboardingStep({ ...skipped, coachSeatDone: true })?.id).toBe('coach_group');
  });

  it('a coach who trains meets the athlete steps, then the coach steps, on one stable bar', () => {
    const trains = coachOnly({ athleteStepsWaived: false });
    const journey = onboardingProgress(trains).map((s) => s.id);
    expect(journey).toEqual([
      'profile_type',
      'connect_provider',
      'parq',
      'coach_proposal',
      'coach_seat',
      'coach_group',
      'coach_invite',
    ]);
    const connected = {
      ...trains,
      parqDone: true,
      needsProviderConnection: false,
      coachProposalDone: true,
    };
    expect(currentOnboardingStep(connected)?.id).toBe('coach_seat');
    expect(onboardingProgress(connected).map((s) => s.id)).toEqual(journey);
  });

  it('a coach who trains still gets the agent step before the seat', () => {
    const connected = coachOnly({
      needsProviderConnection: false,
      justOnboarded: true,
      athleteStepsWaived: false,
      parqDone: true,
    });
    expect(currentOnboardingStep(connected)?.id).toBe('coach_proposal');
  });
});

describe('athleteStepsWaived — durable coach-only record', () => {
  it('is true when either athlete step is recorded not_applicable', () => {
    expect(athleteStepsWaived([{ step_id: 'parq', status: 'not_applicable' }])).toBe(true);
    expect(athleteStepsWaived([{ step_id: 'about_you', status: 'not_applicable' }])).toBe(true);
  });

  it('is false for skipped athlete steps, other steps, and no record', () => {
    expect(athleteStepsWaived([{ step_id: 'parq', status: 'skipped' }])).toBe(false);
    expect(athleteStepsWaived([{ step_id: 'profile_type', status: 'not_applicable' }])).toBe(false);
    expect(athleteStepsWaived(undefined)).toBe(false);
  });

  it('is not a completion: a not_applicable step never reads done', () => {
    expect(isServerStepComplete([{ step_id: 'parq', status: 'not_applicable' }], 'parq')).toBe(false);
  });
});

describe('coach_seat, coach_group, coach_invite — the coach leaves onboarding with a group', () => {
  it('follows the agent step for a coach who has connected, seat first', () => {
    expect(
      currentOnboardingStep(ctx({ coachesOthers: true, justOnboarded: true, coachProposalDone: false }))
        ?.id,
    ).toBe('coach_proposal');
    expect(currentOnboardingStep(ctx({ coachesOthers: true }))?.id).toBe('coach_seat');
    expect(currentOnboardingStep(ctx({ coachesOthers: true, coachSeatDone: true }))?.id).toBe(
      'coach_group',
    );
  });

  it('ends on the invite: once the group is recorded the wizard hands over, with no chat-app step after it', () => {
    expect(
      currentOnboardingStep(ctx({ coachesOthers: true, coachSeatDone: true, coachGroupCreated: true }))?.id,
    ).toBe('coach_invite');
    expect(currentOnboardingStep(ctx({ coachesOthers: true, coachGroupDone: true }))).toBeNull();
    expect(ONBOARDING_STEPS[ONBOARDING_STEPS.length - 1].id).toBe('coach_invite');
  });

  it('is offered to a coach who onboarded before it existed', () => {
    expect(currentOnboardingStep(ctx({ coachesOthers: true, coachSeatDone: true }))?.id).toBe(
      'coach_group',
    );
  });

  it('moves to the invite once the group exists, and ends once it is recorded', () => {
    const made = ctx({ coachesOthers: true, coachSeatDone: true, coachGroupCreated: true });
    expect(currentOnboardingStep(made)?.id).toBe('coach_invite');
    expect(currentOnboardingStep({ ...made, coachGroupDone: true })).toBeNull();
  });

  it('is never an athlete step', () => {
    expect(currentOnboardingStep(ctx())).toBeNull();
    const ids = onboardingProgress(ctx()).map((s) => s.id);
    for (const id of ['coach_seat', 'coach_group', 'coach_invite']) expect(ids).not.toContain(id);
  });

  it('waits for the provider connection', () => {
    expect(
      currentOnboardingStep(ctx({ coachesOthers: true, needsProviderConnection: true }))?.id,
    ).toBe('connect_provider');
  });

  it('is done once finished or put off', () => {
    expect(currentOnboardingStep(ctx({ coachesOthers: true, coachGroupDone: true }))).toBeNull();
  });
});

describe('analytics_consent — asked once of an account no form asked (carnet#1366)', () => {
  const fresh = { needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false };

  it('comes first for a new account that was never asked', () => {
    expect(currentOnboardingStep(ctx({ ...fresh, analyticsConsentAnswered: false }))?.id).toBe(
      'analytics_consent',
    );
  });

  it('is offered once to an onboarded account that was never asked', () => {
    expect(currentOnboardingStep(ctx({ analyticsConsentAnswered: false }))?.id).toBe(
      'analytics_consent',
    );
  });

  it('is never asked again once answered, yes or no', () => {
    expect(currentOnboardingStep(ctx())).toBeNull();
    expect(currentOnboardingStep(ctx(fresh))?.id).toBe('profile_type');
  });

  it('is not asked of an admin, who is outside the flow', () => {
    expect(
      currentOnboardingStep(ctx({ onboardingActive: false, analyticsConsentAnswered: false })),
    ).toBeNull();
  });

  it('is never on the progress bar', () => {
    for (const answered of [true, false]) {
      expect(
        onboardingProgress(ctx({ ...fresh, analyticsConsentAnswered: answered })).map((s) => s.id),
      ).not.toContain('analytics_consent');
    }
  });

  it('leaves an onboarded account reading all done while it is asked', () => {
    const items = onboardingProgress(ctx({ analyticsConsentAnswered: false }));
    expect(items.length).toBeGreaterThan(0);
    expect(items.every((s) => s.status === 'done')).toBe(true);
  });

  it('leaves a new account on its first stage while it is asked', () => {
    const items = onboardingProgress(ctx({ ...fresh, analyticsConsentAnswered: false }));
    expect(items[0]).toMatchObject({ id: 'profile_type', status: 'current' });
  });
});
