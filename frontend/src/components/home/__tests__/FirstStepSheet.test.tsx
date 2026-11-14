// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for FirstStepSheet — Home's « Pour bien démarrer » sheets complete through the onboarding endpoints
// ABOUTME: Each answer writes its step row complete, closes, then runs what waited; closing alone writes nothing (carnet#1530)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { FirstStepSheet, type FirstStepRequest } from '../FirstStepSheet';

const api = vi.hoisted(() => ({
  getParqQuestions: vi.fn(),
  submitParq: vi.fn(),
  saveAboutYou: vi.fn(),
  setOnboardingStep: vi.fn(),
  getCheckIn: vi.fn(),
  updateCheckIn: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  userApi: {
    getParqQuestions: api.getParqQuestions,
    submitParq: api.submitParq,
    saveAboutYou: api.saveAboutYou,
    setOnboardingStep: api.setOnboardingStep,
  },
  athleteApi: { getCheckIn: api.getCheckIn, updateCheckIn: api.updateCheckIn },
}));

function renderSheet(request: FirstStepRequest) {
  const onClose = vi.fn();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <FirstStepSheet request={request} side="right" onClose={onClose} />
    </QueryClientProvider>,
  );
  return { onClose };
}

describe('FirstStepSheet', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getParqQuestions.mockResolvedValue({
      questions: [
        { id: 'heart_condition', text: 'Has a doctor ever said you have a heart condition?' },
        { id: 'chest_pain', text: 'Do you feel pain in your chest?' },
      ],
    });
    api.submitParq.mockResolvedValue({ flags_raised: 0 });
    api.saveAboutYou.mockResolvedValue({ facts_written: 2 });
    api.setOnboardingStep.mockResolvedValue(undefined);
    api.getCheckIn.mockResolvedValue({ cadence: null });
    api.updateCheckIn.mockResolvedValue({ cadence: 'weekly' });
  });

  it('the health check: one tap stores seven-style "no" answers, writes the parq step, then runs what waited', async () => {
    const onDone = vi.fn();
    const { onClose } = renderSheet({ step: 'health_check', onDone });

    expect(screen.getByRole('dialog', { name: 'Health check' })).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('button', { name: 'None of these apply' }));

    await waitFor(() => expect(onDone).toHaveBeenCalledTimes(1));
    expect(api.submitParq).toHaveBeenCalledWith([
      { id: 'heart_condition', yes: false },
      { id: 'chest_pain', yes: false },
    ]);
    expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('parq', 'complete');
    expect(onClose).toHaveBeenCalledTimes(1);
    // No skip in the sheet: closing it is putting it off.
    expect(screen.queryByTestId('parq-skip')).toBeNull();
  });

  it('the health check keeps the sheet and runs nothing when the step could not be written', async () => {
    api.setOnboardingStep.mockRejectedValue(new Error('offline'));
    const onDone = vi.fn();
    const { onClose } = renderSheet({ step: 'health_check', onDone });

    await userEvent.click(await screen.findByRole('button', { name: 'None of these apply' }));

    expect(await screen.findByRole('alert')).toBeInTheDocument();
    expect(onDone).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('the goal and the why go through the about-you endpoint, then the step ticks the row', async () => {
    const { onClose } = renderSheet({ step: 'goal_why' });

    expect(screen.getByRole('dialog', { name: 'Your goal and your why' })).toBeInTheDocument();
    const save = screen.getByTestId('goal-why-save');
    expect(save).toBeDisabled();
    await userEvent.type(screen.getByLabelText('What are you working toward?'), 'Lyon marathon in April');
    await userEvent.click(save);

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(api.saveAboutYou).toHaveBeenCalledWith({
      primary_sport: undefined,
      goal: 'Lyon marathon in April',
      north_star: undefined,
    });
    expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('about_you', 'complete');
  });

  it('the cadence is stored, then the check_in step ticks the row', async () => {
    const { onClose } = renderSheet({ step: 'check_in' });

    await userEvent.click(screen.getByRole('button', { name: 'Every Monday' }));

    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(api.updateCheckIn).toHaveBeenCalledWith('weekly');
    expect(api.setOnboardingStep).toHaveBeenCalledExactlyOnceWith('check_in', 'complete');
  });

  it('closing the sheet writes nothing and runs nothing', async () => {
    const onDone = vi.fn();
    const { onClose } = renderSheet({ step: 'health_check', onDone });

    await userEvent.click(screen.getByRole('button', { name: 'Close' }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(api.setOnboardingStep).not.toHaveBeenCalled();
    expect(onDone).not.toHaveBeenCalled();
  });
});
