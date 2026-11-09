// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Connections calendar choice — shown only with a choice to make, stored, cleared, and a lost calendar said
// ABOUTME: Red if one calendar shows a pointless control, a choice is not stored, or a disconnected choice goes unsaid (carnet#1393)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { PreferredCalendar } from '@pierre/shared-types';
import PreferredCalendarSettings from '../PreferredCalendarSettings';

const api = vi.hoisted(() => ({
  getPreferredCalendar: vi.fn<() => Promise<PreferredCalendar>>(),
  setPreferredCalendar: vi.fn<(provider: string) => Promise<PreferredCalendar>>(),
  clearPreferredCalendar: vi.fn<() => Promise<PreferredCalendar>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getPreferredCalendar: api.getPreferredCalendar,
    setPreferredCalendar: api.setPreferredCalendar,
    clearPreferredCalendar: api.clearPreferredCalendar,
  },
}));

const intervals = { provider: 'intervals_icu', name: 'Intervals.icu' };
const garmin = { provider: 'garmin', name: 'Garmin Connect' };

function renderSettings(connected = 'garmin,intervals_icu') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(
    <QueryClientProvider client={client}>
      <PreferredCalendarSettings connected={connected} />
    </QueryClientProvider>,
  );
  return {
    rerender: (next: string) =>
      view.rerender(
        <QueryClientProvider client={client}>
          <PreferredCalendarSettings connected={next} />
        </QueryClientProvider>,
      ),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('PreferredCalendarSettings', () => {
  it('shows nothing for one calendar and no choice', async () => {
    api.getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals] });
    renderSettings('intervals_icu');

    await waitFor(() => expect(api.getPreferredCalendar).toHaveBeenCalled());
    expect(screen.queryByTestId('preferred-calendar-settings')).toBeNull();
  });

  it('offers none chosen and each connected calendar, showing what the server holds', async () => {
    api.getPreferredCalendar.mockResolvedValue({
      chosen: intervals,
      chosen_connected: true,
      calendars: [intervals, garmin],
    });
    renderSettings();

    const select = await screen.findByTestId('preferred-calendar-select');
    expect(select).toHaveValue('intervals_icu');
    expect(within(select).getAllByRole('option').map((option) => option.getAttribute('value'))).toEqual([
      '',
      'intervals_icu',
      'garmin',
    ]);
    expect(api.setPreferredCalendar).not.toHaveBeenCalled();
  });

  it('stores a choice, and clears it with none chosen', async () => {
    api.getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    api.setPreferredCalendar.mockResolvedValue({ chosen: garmin, chosen_connected: true, calendars: [intervals, garmin] });
    api.clearPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    renderSettings();

    const select = await screen.findByTestId('preferred-calendar-select');
    await userEvent.selectOptions(select, 'garmin');
    expect(api.setPreferredCalendar).toHaveBeenCalledWith('garmin');
    await waitFor(() => expect(select).toHaveValue('garmin'));

    await userEvent.selectOptions(select, '');
    expect(api.clearPreferredCalendar).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(select).toHaveValue(''));
  });

  it('says a recorded calendar is no longer connected, and selects none', async () => {
    api.getPreferredCalendar.mockResolvedValue({ chosen: garmin, chosen_connected: false, calendars: [intervals] });
    renderSettings('intervals_icu');

    const select = await screen.findByTestId('preferred-calendar-select');
    expect(select).toHaveValue('');
    expect(screen.getByTestId('preferred-calendar-disconnected')).toHaveTextContent('Garmin Connect');
  });

  it('says so when a choice could not be saved', async () => {
    api.getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    api.setPreferredCalendar.mockRejectedValue(new Error('offline'));
    renderSettings();

    const select = await screen.findByTestId('preferred-calendar-select');
    await userEvent.selectOptions(select, 'garmin');
    expect(await screen.findByRole('alert')).toBeInTheDocument();
    await waitFor(() => expect(select).toHaveValue(''));
  });

  it('reads the calendars again when a connection is made or removed on the screen', async () => {
    api.getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    const view = renderSettings();
    await screen.findByTestId('preferred-calendar-select');
    const reads = api.getPreferredCalendar.mock.calls.length;

    view.rerender('intervals_icu');
    await waitFor(() => expect(api.getPreferredCalendar.mock.calls.length).toBeGreaterThan(reads));
  });
});
