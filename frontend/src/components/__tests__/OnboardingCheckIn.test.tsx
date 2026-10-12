// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for OnboardingCheckIn — the step asking how often the coach checks in on its own
// ABOUTME: A tap stores the cadence before the flow moves on; never is an answer, a skip stores nothing (carnet#831)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CheckInCadence, CheckInPreference } from '@pierre/shared-types';
import OnboardingCheckIn from '../OnboardingCheckIn';

const api = vi.hoisted(() => ({
  getCheckIn: vi.fn<() => Promise<CheckInPreference>>(),
  updateCheckIn: vi.fn<(cadence: CheckInCadence) => Promise<CheckInPreference>>(),
}));

vi.mock('../../services/api', () => ({
  athleteApi: {
    getCheckIn: api.getCheckIn,
    updateCheckIn: api.updateCheckIn,
  },
}));

function renderStep() {
  const onComplete = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <OnboardingCheckIn onComplete={onComplete} />
    </QueryClientProvider>,
  );
  return { onComplete };
}

describe('OnboardingCheckIn', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getCheckIn.mockResolvedValue({ cadence: null });
  });

  it('asks how often, with the four answers', () => {
    renderStep();
    expect(screen.getByText('How often should your agent check in?')).toBeInTheDocument();
    for (const label of [
      'Every morning',
      'Monday, Wednesday and Friday',
      'Every Monday',
      "Never — I'll start the conversation",
    ]) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });

  it('stores the cadence tapped, then completes the step', async () => {
    api.updateCheckIn.mockResolvedValue({ cadence: 'three_per_week' });
    const { onComplete } = renderStep();

    await userEvent.click(screen.getByRole('button', { name: 'Monday, Wednesday and Friday' }));

    expect(api.updateCheckIn).toHaveBeenCalledWith('three_per_week');
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith('complete'));
  });

  it('takes never as an answer like the others', async () => {
    api.updateCheckIn.mockResolvedValue({ cadence: 'off' });
    const { onComplete } = renderStep();

    await userEvent.click(screen.getByRole('button', { name: "Never — I'll start the conversation" }));

    expect(api.updateCheckIn).toHaveBeenCalledWith('off');
    await waitFor(() => expect(onComplete).toHaveBeenCalledWith('complete'));
  });

  it('stays on the step and says so when the choice could not be stored', async () => {
    api.updateCheckIn.mockRejectedValue(new Error('offline'));
    const { onComplete } = renderStep();

    await userEvent.click(screen.getByRole('button', { name: 'Every morning' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Your check-in choice could not be saved. Try again.',
    );
    expect(onComplete).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Every morning' })).toBeEnabled();
  });

  it('a skip stores nothing and moves on', async () => {
    const { onComplete } = renderStep();
    await userEvent.click(screen.getByRole('button', { name: 'Skip for now' }));
    expect(api.updateCheckIn).not.toHaveBeenCalled();
    expect(onComplete).toHaveBeenCalledWith('skipped');
  });
});
