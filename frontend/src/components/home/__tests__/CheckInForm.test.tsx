// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for CheckInForm — Home's « Ton point du jour » sheet, asking how often the agent checks in on its own
// ABOUTME: A tap stores the cadence before the host ticks the row; never is an answer, a refusal keeps the sheet (carnet#831, carnet#1530)

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CheckInCadence, CheckInPreference } from '@pierre/shared-types';
import { CheckInForm } from '../CheckInForm';

const api = vi.hoisted(() => ({
  getCheckIn: vi.fn<() => Promise<CheckInPreference>>(),
  updateCheckIn: vi.fn<(cadence: CheckInCadence) => Promise<CheckInPreference>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getCheckIn: api.getCheckIn,
    updateCheckIn: api.updateCheckIn,
  },
}));

function renderForm(onChosen = vi.fn<() => Promise<void>>().mockResolvedValue(undefined)) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <CheckInForm onChosen={onChosen} />
    </QueryClientProvider>,
  );
  return { onChosen };
}

describe('CheckInForm', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    api.getCheckIn.mockResolvedValue({ cadence: null });
  });

  it('offers the four answers', () => {
    renderForm();
    for (const label of [
      'Every morning',
      'Monday, Wednesday and Friday',
      'Every Monday',
      "Never — I'll start the conversation",
    ]) {
      expect(screen.getByRole('button', { name: label })).toBeInTheDocument();
    }
  });

  it('stores the cadence tapped, then hands over to tick the row', async () => {
    api.updateCheckIn.mockResolvedValue({ cadence: 'three_per_week' });
    const { onChosen } = renderForm();

    await userEvent.click(screen.getByRole('button', { name: 'Monday, Wednesday and Friday' }));

    expect(api.updateCheckIn).toHaveBeenCalledWith('three_per_week');
    await waitFor(() => expect(onChosen).toHaveBeenCalledTimes(1));
  });

  it('takes never as an answer like the others', async () => {
    api.updateCheckIn.mockResolvedValue({ cadence: 'off' });
    const { onChosen } = renderForm();

    await userEvent.click(screen.getByRole('button', { name: "Never — I'll start the conversation" }));

    expect(api.updateCheckIn).toHaveBeenCalledWith('off');
    await waitFor(() => expect(onChosen).toHaveBeenCalledTimes(1));
  });

  it('stays and says so when the choice could not be stored', async () => {
    api.updateCheckIn.mockRejectedValue(new Error('offline'));
    const { onChosen } = renderForm();

    await userEvent.click(screen.getByRole('button', { name: 'Every morning' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Your check-in choice could not be saved. Try again.',
    );
    expect(onChosen).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: 'Every morning' })).toBeEnabled();
  });

  it('stays when the row could not be ticked, so the count never lies', async () => {
    api.updateCheckIn.mockResolvedValue({ cadence: 'daily' });
    renderForm(vi.fn<() => Promise<void>>().mockRejectedValue(new Error('offline')));

    await userEvent.click(screen.getByRole('button', { name: 'Every morning' }));

    expect(await screen.findByRole('alert')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Every morning' })).toBeEnabled();
  });
});
