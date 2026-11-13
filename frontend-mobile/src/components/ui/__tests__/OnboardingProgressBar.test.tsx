// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProgressBar — the segmented hairline under an onboarding screen's header
// ABOUTME: Pins one segment per step, the dashed skipped segment, and "Step N of M: label" as caption and accessible name

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { OnboardingProgressItem, OnboardingStepStatus } from '@pierre/shared-constants';
import { OnboardingProgressBar } from '../OnboardingProgressBar';

/** An athlete's journey, steps by status: about-you put off, the health check current. */
const STEPS: OnboardingProgressItem[] = [
  { id: 'profile_type', labelKey: 'onboarding.stepProfile', status: 'done' },
  { id: 'about_you', labelKey: 'onboarding.stepAboutTraining', status: 'skipped' },
  { id: 'parq', labelKey: 'onboarding.stepHealthCheck', status: 'current' },
  { id: 'check_in', labelKey: 'onboarding.stepCheckIn', status: 'upcoming' },
  { id: 'connect_provider', labelKey: 'onboarding.stepConnect', status: 'upcoming' },
];

/** The segments and the caption are hidden from assistive tech: the bar's own name says it all. */
const HIDDEN = { includeHiddenElements: true } as const;
const segment = (id: string) => screen.getByTestId(`onboarding-progress-segment-${id}`, HIDDEN);

describe('OnboardingProgressBar', () => {
  it('renders nothing for an empty journey', () => {
    const { toJSON } = render(<OnboardingProgressBar steps={[]} />);
    expect(toJSON()).toBeNull();
  });

  it('draws one segment per step of the journey, skipped or not', () => {
    render(<OnboardingProgressBar steps={STEPS} />);
    for (const step of STEPS) {
      expect(segment(step.id)).toBeTruthy();
    }
  });

  it('fills what is reached, dashes what was put off, leaves the rest as the track', () => {
    render(<OnboardingProgressBar steps={STEPS} />);
    const look: Record<OnboardingStepStatus, string> = {
      done: 'bg-primary',
      current: 'bg-primary',
      skipped: 'border-dashed',
      upcoming: 'bg-outline-variant',
    };
    for (const step of STEPS) {
      expect(segment(step.id).props.className).toContain(look[step.status]);
    }
  });

  it('says "Step N of M: <label>" as its accessible name and as the caption under it', () => {
    render(<OnboardingProgressBar steps={STEPS} testID="bar" />);
    const bar = screen.getByTestId('bar');
    expect(bar.props.accessibilityLabel).toBe('Step 3 of 5: Health');
    expect(bar.props.accessibilityValue).toEqual({ min: 0, max: 5, now: 3 });
    expect(screen.getByTestId('onboarding-progress-caption', HIDDEN)).toHaveTextContent(
      'Step 3 of 5: Health',
    );
  });

  it('with every step behind the user, reads full and names no step', () => {
    const allBehind = STEPS.map((step) => ({ ...step, status: 'done' as const }));
    render(<OnboardingProgressBar steps={allBehind} testID="bar" />);
    expect(screen.getByTestId('bar').props.accessibilityValue).toEqual({ min: 0, max: 5, now: 5 });
    expect(screen.queryByTestId('onboarding-progress-caption', HIDDEN)).toBeNull();
  });
});
