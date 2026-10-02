// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProfileType — the athlete / coach / both onboarding step
// ABOUTME: Verifies both coach choices persist coaching_persona=coach and only the coach-only one drops the athlete steps

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

  it('renders the athlete, coach, and coach-who-trains choices', () => {
    renderStep();
    expect(screen.getByText("I'm an athlete")).toBeInTheDocument();
    expect(screen.getByText('I coach others')).toBeInTheDocument();
    expect(screen.getByText('I coach and I train')).toBeInTheDocument();
  });

  it('athlete choice completes as someone who trains, without writing a persona', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText("I'm an athlete"));
    expect(onComplete).toHaveBeenCalledWith({ trains: true });
    expect(setCoachingPersonaMock).not.toHaveBeenCalled();
  });

  it('coach choice persists coaching_persona=coach and leaves the athlete steps out', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach others'));
    await waitFor(() => expect(setCoachingPersonaMock).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: false }));
  });

  it('coach-who-trains choice persists coaching_persona=coach and keeps the athlete steps', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach and I train'));
    await waitFor(() => expect(setCoachingPersonaMock).toHaveBeenCalledWith('coach'));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: true }));
  });

  it('stays on the step with an error when the persona write fails, and lets the user retry', async () => {
    setCoachingPersonaMock.mockRejectedValueOnce(new Error('network'));
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach others'));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      "We couldn't save your choice. Please try again.",
    );
    expect(onComplete).not.toHaveBeenCalled();

    await userEvent.click(screen.getByText('I coach others'));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: false }));
  });
});
