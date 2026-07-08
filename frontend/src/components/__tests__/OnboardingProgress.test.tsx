// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProgress — the labeled step-dots onboarding progress indicator
// ABOUTME: Verifies labels render and the current step is announced to assistive tech via sr-only text

import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import OnboardingProgress from '../OnboardingProgress';
import type { OnboardingProgressItem } from '../../onboarding/steps';

const STEPS: OnboardingProgressItem[] = [
  { id: 'profile_type', label: 'About you', status: 'done' },
  { id: 'connect_provider', label: 'Connect', status: 'current' },
  { id: 'coach_proposal', label: 'Coach', status: 'upcoming' },
];

describe('OnboardingProgress', () => {
  it('renders every step label', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(screen.getByText('About you')).toBeInTheDocument();
    expect(screen.getByText('Connect')).toBeInTheDocument();
    expect(screen.getByText('Coach')).toBeInTheDocument();
  });

  it('announces the current step (1-indexed) to assistive tech', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(screen.getByText('Step 2 of 3: Connect')).toBeInTheDocument();
  });

  it('is exposed as a labeled navigation landmark', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(screen.getByRole('navigation', { name: 'Onboarding progress' })).toBeInTheDocument();
  });
});
