// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the onboarding step registry — decision engine + progress status computation
// ABOUTME: Pins currentOnboardingStep as a faithful translation of the former App.tsx gate chain (all scenarios)

import { describe, it, expect } from 'vitest';
import {
  currentOnboardingStep,
  isServerStepComplete,
  onboardingProgress,
  ONBOARDING_STEPS,
  type OnboardingContext,
} from '../steps';

/** A fully-onboarded, returning-user baseline; each test overrides only what it exercises. */
function ctx(overrides: Partial<OnboardingContext> = {}): OnboardingContext {
  return {
    onboardingActive: true,
    needsProviderConnection: false,
    skippedProvider: false,
    justOnboarded: false,
    profileTypeChosen: true,
    coachProposalDone: true,
    // The pre-connect questionnaire steps default to done in this baseline, so a
    // test exercising a later step isn't intercepted by them.
    aboutYouDone: true,
    parqDone: true,
    // Messaging defaults: no channels configured, so the messaging steps are
    // inapplicable + auto-complete unless a test opts in with a positive count.
    messagingAvailableCount: 0,
    messagingChannelChosen: false,
    messagingChannelDone: false,
    messagingConfigureDone: false,
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

  it('profile chosen, about-you outstanding → about_you (before the provider gate)', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: true,
          profileTypeChosen: true,
          aboutYouDone: false,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('about_you');
  });

  it('about-you done, PAR-Q outstanding → parq (still before the provider gate)', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: true,
          profileTypeChosen: true,
          aboutYouDone: true,
          parqDone: false,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('parq');
  });

  it('skipping about-you and PAR-Q still lands on connect_provider', () => {
    expect(
      currentOnboardingStep(
        ctx({
          needsProviderConnection: true,
          profileTypeChosen: true,
          aboutYouDone: true,
          parqDone: true,
          coachProposalDone: false,
        }),
      )?.id,
    ).toBe('connect_provider');
  });

  it('just connected this session (needs=false, justOnboarded) → coach_proposal', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
      )?.id,
    ).toBe('coach_proposal');
  });

  it('returning already-onboarded user → null (dashboard)', () => {
    expect(currentOnboardingStep(ctx())).toBeNull();
  });

  it('returning user with cleared localStorage never re-sees profile_type (needs=false)', () => {
    // Regression guard: profile_type is gated on needs===true, so a
    // long-connected user with no local flags must NOT be pulled back into it.
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, profileTypeChosen: false, coachProposalDone: false }),
      ),
    ).toBeNull();
  });

  it('"continue without connecting" (skipped) → null (dashboard)', () => {
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: true, skippedProvider: true, profileTypeChosen: false }),
      ),
    ).toBeNull();
  });

  it('admin / inactive user (onboardingActive=false, needs=undefined) → null', () => {
    expect(
      currentOnboardingStep(
        ctx({ onboardingActive: false, needsProviderConnection: undefined }),
      ),
    ).toBeNull();
  });

  it('coach_proposal does not fire without the in-session justOnboarded transition', () => {
    // needs=false but justOnboarded=false (plain login) → straight to dashboard.
    expect(
      currentOnboardingStep(
        ctx({ needsProviderConnection: false, justOnboarded: false, coachProposalDone: false }),
      ),
    ).toBeNull();
  });
});

describe('onboardingProgress — stable position-based labeled sequence', () => {
  it('shows only the steps this user will actually meet', () => {
    // A typical self-serve user: their own tenant has no messaging channels, so
    // the two messaging steps can never run. The bar used to list them anyway,
    // promising a five-step journey that ran in two.
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
    );
    expect(items.map((i) => i.labelKey)).toEqual([
      'onboarding.stepAboutYou',
      'onboarding.stepAboutTraining',
      'onboarding.stepHealthCheck',
      'onboarding.stepConnect',
    ]);
  });

  it('keeps canonical order — the journey is a subset, never a reshuffle', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
    );
    const canonical = ONBOARDING_STEPS.map((s) => s.id);
    const positions = items.map((i) => canonical.indexOf(i.id));
    expect(positions).toEqual([...positions].sort((a, b) => a - b));
    expect(positions.every((p) => p >= 0)).toBe(true);
  });

  it('includes the messaging steps once the tenant actually has channels', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        justOnboarded: true,
        coachProposalDone: false,
        messagingAvailableCount: 3,
      }),
    );
    expect(items.map((i) => i.id)).toContain('messaging_channel');
  });

  /**
   * The contract `onboardingProgress` actually promises is *positional*: the
   * current step is `current`, everything before it is `done`, everything after
   * is `upcoming`. Asserting that invariant — rather than a snapshot of the
   * pipeline's length — is what keeps these tests meaningful when a step is
   * added. The label test above still pins the pipeline itself, so a change to
   * it remains deliberate rather than silent.
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
        aboutYouDone: false,
        parqDone: false,
        coachProposalDone: false,
      }),
    );
    expectPositional(items, 'profile_type');
    expect(items[0].status).toBe('current');
  });

  it('on about_you: profile_type behind it is done', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: true,
        profileTypeChosen: true,
        aboutYouDone: false,
        parqDone: false,
        coachProposalDone: false,
      }),
    );
    expectPositional(items, 'about_you');
  });

  it('on parq: the two questionnaire steps behind it are done', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: true,
        profileTypeChosen: true,
        aboutYouDone: true,
        parqDone: false,
        coachProposalDone: false,
      }),
    );
    expectPositional(items, 'parq');
  });

  it('on connect_provider: every pre-connect step is behind it', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: true, coachProposalDone: false }),
    );
    expectPositional(items, 'connect_provider');
  });

  it('on coach_proposal: the provider gate is behind it', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
    );
    expectPositional(items, 'coach_proposal');
  });

  it('on messaging_channel (multiple channels)', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        justOnboarded: true,
        coachProposalDone: true,
        messagingAvailableCount: 3,
      }),
    );
    expectPositional(items, 'messaging_channel');
  });

  it('on messaging_configure (channel chosen): it is last and everything precedes it', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        justOnboarded: true,
        coachProposalDone: true,
        messagingAvailableCount: 3,
        messagingChannelChosen: true,
        messagingChannelDone: true,
      }),
    );
    expectPositional(items, 'messaging_configure');
    expect(items[items.length - 1].status).toBe('current');
  });
});

describe('currentOnboardingStep — messaging steps', () => {
  // No `justOnboarded`: messaging is reachable for an already-connected returning
  // user who hasn't set it up (unlike coach-proposal), so these omit that flag.
  const afterCoach = { coachProposalDone: true } as const;

  it('after coach with multiple channels → messaging_channel (picker)', () => {
    expect(
      currentOnboardingStep(ctx({ ...afterCoach, messagingAvailableCount: 3 }))?.id,
    ).toBe('messaging_channel');
  });

  it('single channel auto-skips the picker → messaging_configure', () => {
    expect(
      currentOnboardingStep(
        ctx({ ...afterCoach, messagingAvailableCount: 1, messagingChannelChosen: true }),
      )?.id,
    ).toBe('messaging_configure');
  });

  it('once a channel is chosen → messaging_configure', () => {
    expect(
      currentOnboardingStep(
        ctx({
          ...afterCoach,
          messagingAvailableCount: 3,
          messagingChannelChosen: true,
          messagingChannelDone: true,
        }),
      )?.id,
    ).toBe('messaging_configure');
  });

  it('no channels configured → skip messaging entirely → null (dashboard)', () => {
    expect(
      currentOnboardingStep(ctx({ ...afterCoach, messagingAvailableCount: 0 })),
    ).toBeNull();
  });

  it('messaging configured + done → null (dashboard)', () => {
    expect(
      currentOnboardingStep(
        ctx({
          ...afterCoach,
          messagingAvailableCount: 3,
          messagingChannelChosen: true,
          messagingChannelDone: true,
          messagingConfigureDone: true,
        }),
      ),
    ).toBeNull();
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
