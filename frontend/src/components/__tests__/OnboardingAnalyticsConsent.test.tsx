// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingAnalyticsConsent — the usage-analytics question for an account no form asked
// ABOUTME: Yes and No are one tap each, the answer is stored before moving on, a failed save stays to retry (carnet#1366)

import { describe, it, expect, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { ANALYTICS_POLICY_URL } from '@pierre/shared-constants';
import OnboardingAnalyticsConsent from '../OnboardingAnalyticsConsent';

const ACCEPT = 'Yes, share usage analytics';
const DECLINE = 'No thanks';

describe('OnboardingAnalyticsConsent', () => {
  it('asks the question with both answers and the policy link', () => {
    render(<OnboardingAnalyticsConsent onAnswer={vi.fn()} />);

    expect(screen.getByText('Help improve Dravr?')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: ACCEPT })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: DECLINE })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'How we use usage analytics' })).toHaveAttribute(
      'href',
      ANALYTICS_POLICY_URL,
    );
  });

  it('gives Yes and No the same weight', () => {
    render(<OnboardingAnalyticsConsent onAnswer={vi.fn()} />);

    expect(screen.getByRole('button', { name: ACCEPT }).className).toBe(
      screen.getByRole('button', { name: DECLINE }).className,
    );
  });

  it.each([
    [ACCEPT, true],
    [DECLINE, false],
  ])('stores "%s" as %s', async (label, enabled) => {
    const onAnswer = vi.fn().mockResolvedValue(undefined);
    render(<OnboardingAnalyticsConsent onAnswer={onAnswer} />);

    await userEvent.click(screen.getByRole('button', { name: label }));

    expect(onAnswer).toHaveBeenCalledTimes(1);
    expect(onAnswer).toHaveBeenCalledWith(enabled);
  });

  it('keeps the person on the question when the answer is not saved', async () => {
    const onAnswer = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue(undefined);
    render(<OnboardingAnalyticsConsent onAnswer={onAnswer} />);

    await userEvent.click(screen.getByRole('button', { name: DECLINE }));

    expect(await screen.findByRole('alert')).toHaveTextContent("Your answer wasn't saved. Try again.");
    const retry = screen.getByRole('button', { name: DECLINE });
    await waitFor(() => expect(retry).toBeEnabled());
    await userEvent.click(retry);
    expect(onAnswer).toHaveBeenCalledTimes(2);
  });
});
