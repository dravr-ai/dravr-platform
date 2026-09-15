// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProgressBar — the filling hairline under an onboarding screen's header
// ABOUTME: Pins the fill fraction by position (not by isComplete) and the "Step N of M: label" accessible name

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { OnboardingProgressItem, OnboardingStepId } from '@pierre/shared-constants';
import { OnboardingProgressBar } from '../OnboardingProgressBar';

/** Real step ids and label keys from the shared registry, in canonical order. */
const STEP_IDS: OnboardingStepId[] = [
  'profile_type',
  'about_you',
  'parq',
  'connect_provider',
  'coach_proposal',
];
const LABEL_KEYS = [
  'onboarding.stepAboutYou',
  'onboarding.stepAboutTraining',
  'onboarding.stepHealthCheck',
  'onboarding.stepConnect',
  'onboarding.stepAgent',
];

const steps = (currentIndex: number, total: number): OnboardingProgressItem[] =>
  Array.from({ length: total }, (_, i) => ({
    id: STEP_IDS[i],
    labelKey: LABEL_KEYS[i],
    status: i < currentIndex ? 'done' : i === currentIndex ? 'current' : 'upcoming',
  }));

describe('OnboardingProgressBar', () => {
  it('renders nothing for an empty journey', () => {
    const { toJSON } = render(<OnboardingProgressBar steps={[]} />);
    expect(toJSON()).toBeNull();
  });

  it('fills to (current index + 1) of the total, not to the done count', () => {
    // 5 steps, step 3 (index 2, "Health check") current: two done, this one
    // current — the fraction is 3 of 5, never 2 of 5 (isComplete's count).
    render(<OnboardingProgressBar steps={steps(2, 5)} testID="bar" />);
    const track = screen.getByTestId('bar');
    const fillWidth = track.props.children.props.style.width as string;
    expect(fillWidth).toBe(`${(3 / 5) * 100}%`);
  });

  it('fills all the way once every step is behind the user', () => {
    render(<OnboardingProgressBar steps={steps(5, 5)} testID="bar" />);
    const track = screen.getByTestId('bar');
    const fillWidth = track.props.children.props.style.width as string;
    expect(fillWidth).toBe('100%');
  });

  it('carries "Step N of M: <label>" as its accessible name', () => {
    render(<OnboardingProgressBar steps={steps(1, 3)} testID="bar" />);
    const track = screen.getByTestId('bar');
    expect(track.props.accessibilityLabel).toBe('Step 2 of 3: About your training');
  });
});
