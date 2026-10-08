// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingProfileType — the athlete / coach / both onboarding step
// ABOUTME: Verifies each choice records the coaching role (never the persona) and only coach-only drops the athlete steps

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import OnboardingProfileType from '../OnboardingProfileType';

const { setCoachingRoleMock } = vi.hoisted(() => ({
  setCoachingRoleMock: vi.fn(),
}));

vi.mock('../../services/api', () => ({
  userApi: { setCoachingRole: setCoachingRoleMock },
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
    setCoachingRoleMock.mockReset();
    setCoachingRoleMock.mockResolvedValue({ coaches_others: true });
  });

  it('renders the athlete, coach, and coach-who-trains choices', () => {
    renderStep();
    expect(screen.getByText("I'm an athlete")).toBeInTheDocument();
    expect(screen.getByText('I coach others')).toBeInTheDocument();
    expect(screen.getByText('I coach and I train')).toBeInTheDocument();
  });

  it('athlete choice records the role as off and completes as someone who trains', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText("I'm an athlete"));
    await waitFor(() => expect(setCoachingRoleMock).toHaveBeenCalledWith(false));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: true }));
  });

  it('coach choice records the coach role and leaves the athlete steps out', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach others'));
    await waitFor(() => expect(setCoachingRoleMock).toHaveBeenCalledWith(true));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: false }));
  });

  it('coach-who-trains choice records the coach role and keeps the athlete steps', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByText('I coach and I train'));
    await waitFor(() => expect(setCoachingRoleMock).toHaveBeenCalledWith(true));
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith({ trains: true }));
  });

  it('stays on the step with an error when the role write fails, and lets the user retry', async () => {
    setCoachingRoleMock.mockRejectedValueOnce(new Error('network'));
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
