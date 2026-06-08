// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProfileType — the athlete-vs-coach onboarding step
// ABOUTME: Verifies the coach choice persists coaching_persona=coach and both choices complete

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingProfileType from '../OnboardingProfileType';

const { setCoachingPersonaMock } = vi.hoisted(() => ({
  setCoachingPersonaMock: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  userApi: { setCoachingPersona: setCoachingPersonaMock },
}));

function renderStep() {
  const onComplete = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingProfileType userDisplayName="Jean" onComplete={onComplete} />
    </QueryClientProvider>
  );
  return { onComplete };
}

describe('OnboardingProfileType', () => {
  beforeEach(() => {
    setCoachingPersonaMock.mockReset();
    setCoachingPersonaMock.mockResolvedValue({ persona: 'coach' });
  });

  it('renders both the athlete and coach choices', () => {
    renderStep();
    expect(screen.getByText("I'm an athlete")).toBeInTheDocument();
    expect(screen.getByText('I coach others')).toBeInTheDocument();
  });

  it('athlete choice completes without writing a persona', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText("I'm an athlete"));
    expect(onComplete).toHaveBeenCalledTimes(1);
    expect(setCoachingPersonaMock).not.toHaveBeenCalled();
  });

  it('coach choice persists coaching_persona=coach then completes', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach others'));
    await waitFor(() => expect(setCoachingPersonaMock).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
  });

  it('still completes when the persona write fails', async () => {
    setCoachingPersonaMock.mockRejectedValueOnce(new Error('network'));
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach others'));
    await waitFor(() => expect(onComplete).toHaveBeenCalledTimes(1));
  });
});
