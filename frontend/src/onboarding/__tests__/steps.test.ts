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
  it('exposes the full canonical pipeline in order, regardless of phase', () => {
    const items = onboardingProgress(ctx({ needsProviderConnection: true, profileTypeChosen: false }));
    expect(items.map((i) => i.id)).toEqual(ONBOARDING_STEPS.map((s) => s.id));
    expect(items.map((i) => i.label)).toEqual(['About you', 'Connect', 'Coach', 'Chat app', 'Link']);
  });

  it('on profile_type: first current, rest upcoming (messaging not prematurely done)', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: false, coachProposalDone: false }),
    );
    expect(items.map((i) => i.status)).toEqual([
      'current',
      'upcoming',
      'upcoming',
      'upcoming',
      'upcoming',
    ]);
  });

  it('on connect_provider: first done, second current', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: true, profileTypeChosen: true, coachProposalDone: false }),
    );
    expect(items.map((i) => i.status)).toEqual(['done', 'current', 'upcoming', 'upcoming', 'upcoming']);
  });

  it('on coach_proposal: first two done, third current', () => {
    const items = onboardingProgress(
      ctx({ needsProviderConnection: false, justOnboarded: true, coachProposalDone: false }),
    );
    expect(items.map((i) => i.status)).toEqual(['done', 'done', 'current', 'upcoming', 'upcoming']);
  });

  it('on messaging_channel (multiple channels): fourth current', () => {
    const items = onboardingProgress(
      ctx({
        needsProviderConnection: false,
        justOnboarded: true,
        coachProposalDone: true,
        messagingAvailableCount: 3,
      }),
    );
    expect(items.map((i) => i.status)).toEqual(['done', 'done', 'done', 'current', 'upcoming']);
  });

  it('on messaging_configure (channel chosen): fifth current', () => {
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
    expect(items.map((i) => i.status)).toEqual(['done', 'done', 'done', 'done', 'current']);
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
