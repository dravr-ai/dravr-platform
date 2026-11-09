// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Connections calendar choice — shown only with a choice to make, stored, cleared, a lost one said
// ABOUTME: The athlete API is mocked; assertions read the rows, the selected one and the calls (carnet#1393)

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { PreferredCalendarSection } from '../PreferredCalendarSection';
import { athleteApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  athleteApi: {
    getPreferredCalendar: jest.fn(),
    setPreferredCalendar: jest.fn(),
    clearPreferredCalendar: jest.fn(),
  },
}));

const getPreferredCalendar = athleteApi.getPreferredCalendar as jest.Mock;
const setPreferredCalendar = athleteApi.setPreferredCalendar as jest.Mock;
const clearPreferredCalendar = athleteApi.clearPreferredCalendar as jest.Mock;

const intervals = { provider: 'intervals_icu', name: 'Intervals.icu' };
const garmin = { provider: 'garmin', name: 'Garmin Connect' };

function renderSection(connected = 'garmin,intervals_icu') {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <PreferredCalendarSection connected={connected} />
    </QueryClientProvider>,
  );
}

describe('PreferredCalendarSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('shows nothing for one calendar and no choice', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals] });
    renderSection('intervals_icu');

    await waitFor(() => expect(getPreferredCalendar).toHaveBeenCalled());
    expect(screen.queryByTestId('preferred-calendar-section')).toBeNull();
  });

  it('offers none chosen and each connected calendar, selecting what the server holds', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: intervals, chosen_connected: true, calendars: [intervals, garmin] });
    renderSection();

    await waitFor(() =>
      expect(screen.getByTestId('preferred-calendar-option-intervals_icu').props.accessibilityState).toMatchObject({
        selected: true,
      }),
    );
    expect(screen.getByTestId('preferred-calendar-option-none').props.accessibilityState).toMatchObject({
      selected: false,
    });
    expect(screen.getByTestId('preferred-calendar-option-garmin')).toBeTruthy();
    expect(setPreferredCalendar).not.toHaveBeenCalled();
  });

  it('stores a choice, and clears it with none chosen', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    setPreferredCalendar.mockResolvedValue({ chosen: garmin, chosen_connected: true, calendars: [intervals, garmin] });
    clearPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    renderSection();

    await waitFor(() => expect(screen.getByTestId('preferred-calendar-option-garmin')).toBeTruthy());
    fireEvent.press(screen.getByTestId('preferred-calendar-option-garmin'));
    await waitFor(() => expect(setPreferredCalendar).toHaveBeenCalledWith('garmin'));
    await waitFor(() =>
      expect(screen.getByTestId('preferred-calendar-option-garmin').props.accessibilityState).toMatchObject({
        selected: true,
        disabled: false,
      }),
    );

    fireEvent.press(screen.getByTestId('preferred-calendar-option-none'));
    await waitFor(() => expect(clearPreferredCalendar).toHaveBeenCalledTimes(1));
  });

  it('says a recorded calendar is no longer connected, and selects none', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: garmin, chosen_connected: false, calendars: [intervals] });
    renderSection('intervals_icu');

    await waitFor(() => expect(screen.getByTestId('preferred-calendar-disconnected')).toBeTruthy());
    expect(screen.getByTestId('preferred-calendar-option-none').props.accessibilityState).toMatchObject({
      selected: true,
    });
  });

  it('says so when a choice could not be saved', async () => {
    getPreferredCalendar.mockResolvedValue({ chosen: null, chosen_connected: false, calendars: [intervals, garmin] });
    setPreferredCalendar.mockRejectedValue(new Error('offline'));
    renderSection();

    await waitFor(() => expect(screen.getByTestId('preferred-calendar-option-garmin')).toBeTruthy());
    fireEvent.press(screen.getByTestId('preferred-calendar-option-garmin'));
    await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
  });
});
