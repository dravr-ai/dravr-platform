// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings check-in choice — it reads and writes the server cadence, never reads as off, and says when a save fails
// ABOUTME: Red if a choice is not stored, an athlete who never chose is shown a cadence, or a refused save goes unsaid (carnet#831)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CheckInCadence, CheckInPreference } from '@pierre/shared-types';
import CheckInSettings from '../CheckInSettings';

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

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <CheckInSettings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('CheckInSettings', () => {
  it('offers the four cadences and shows the one stored', async () => {
    api.getCheckIn.mockResolvedValue({ cadence: 'three_per_week' });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Choose how often your agent checks in' });
    await waitFor(() => expect(select).toBeEnabled());
    expect(select).toHaveValue('three_per_week');
    expect(screen.getAllByRole('option').map((option) => option.textContent)).toEqual([
      'Every morning',
      'Monday, Wednesday and Friday',
      'Every Monday',
      "Never — I'll start the conversation",
    ]);
  });

  it('shows an athlete who never chose as never, which is what they get', async () => {
    api.getCheckIn.mockResolvedValue({ cadence: null });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Choose how often your agent checks in' });
    await waitFor(() => expect(select).toBeEnabled());
    expect(select).toHaveValue('off');
  });

  it('stores a choice', async () => {
    api.getCheckIn.mockResolvedValue({ cadence: 'off' });
    api.updateCheckIn.mockResolvedValue({ cadence: 'daily' });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Choose how often your agent checks in' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'daily');

    expect(api.updateCheckIn).toHaveBeenCalledWith('daily');
    await waitFor(() => expect(select).toHaveValue('daily'));
  });

  it('says so when a choice could not be saved, and keeps what the server holds', async () => {
    api.getCheckIn.mockResolvedValue({ cadence: 'weekly' });
    api.updateCheckIn.mockRejectedValue(new Error('offline'));
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Choose how often your agent checks in' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'daily');

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Your check-in choice could not be saved. Try again.',
    );
    await waitFor(() => expect(select).toHaveValue('weekly'));
  });
});
