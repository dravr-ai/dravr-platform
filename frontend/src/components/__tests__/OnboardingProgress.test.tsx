// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProgress — the labeled step-dots onboarding progress indicator
// ABOUTME: Verifies labels, the skipped ring, readable upcoming ink, and position + total in the landmark's name

import { describe, it, expect } from 'vitest';
import { render, screen } from '@testing-library/react';
import OnboardingProgress from '../OnboardingProgress';
import type { OnboardingProgressItem } from '../../onboarding/steps';

const STEPS: OnboardingProgressItem[] = [
  { id: 'profile_type', labelKey: 'onboarding.stepProfile', status: 'done' },
  { id: 'connect_provider', labelKey: 'humanCoach.stepSeat', status: 'skipped' },
  { id: 'coach_group', labelKey: 'onboarding.stepGroup', status: 'current' },
  { id: 'coach_proposal', labelKey: 'onboarding.stepAgent', status: 'upcoming' },
];

/** The list item the bar draws for one step. */
function item(container: HTMLElement, status: OnboardingProgressItem['status']): HTMLElement {
  const li = container.querySelector<HTMLElement>(`li[data-status="${status}"]`);
  if (!li) throw new Error(`no ${status} step on the bar`);
  return li;
}

describe('OnboardingProgress', () => {
  it('renders every step label', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(screen.getByText('Profile')).toBeInTheDocument();
    expect(screen.getByText('Coach seat')).toBeInTheDocument();
    expect(screen.getByText('Your group')).toBeInTheDocument();
    expect(screen.getByText('Your team')).toBeInTheDocument();
  });

  it('names the landmark with the position, the total and the step', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(
      screen.getByRole('navigation', { name: 'Step 3 of 4: Your group' }),
    ).toBeInTheDocument();
  });

  it('says the position visibly: with the step under 1024px, alone beside the labels above it', () => {
    render(<OnboardingProgress steps={STEPS} />);
    expect(screen.getByTestId('onboarding-current-step')).toHaveTextContent('Step 3 of 4: Your group');
    expect(screen.getByTestId('onboarding-step-counter')).toHaveTextContent(/^Step 3 of 4$/);
  });

  it('keeps a skipped step on the bar as a dashed ring, an upcoming one as a whole ring', () => {
    const { container } = render(<OnboardingProgress steps={STEPS} />);
    expect(item(container, 'skipped').querySelector('.border-dashed')).not.toBeNull();
    expect(item(container, 'upcoming').querySelector('.border-dashed')).toBeNull();
    expect(item(container, 'upcoming').querySelector('.border-solid')).not.toBeNull();
  });

  it('draws every label but the current one in full-strength secondary ink', () => {
    const { container } = render(<OnboardingProgress steps={STEPS} />);
    for (const status of ['done', 'skipped', 'upcoming'] as const) {
      const label = item(container, status).querySelector('span.text-xs');
      expect(label?.className).toMatch(/(^|\s)text-on-surface-variant(\s|$)/);
    }
    expect(item(container, 'current').querySelector('span.text-xs')?.className).toMatch(
      /(^|\s)text-on-surface(\s|$)/,
    );
  });

  it('with no step current, keeps its plain name and says no position', () => {
    const allBehind = STEPS.map((s) => ({ ...s, status: 'done' as const }));
    render(<OnboardingProgress steps={allBehind} />);
    expect(screen.getByRole('navigation', { name: 'Onboarding progress' })).toBeInTheDocument();
    expect(screen.queryByTestId('onboarding-current-step')).toBeNull();
    expect(screen.queryByTestId('onboarding-step-counter')).toBeNull();
  });
});
