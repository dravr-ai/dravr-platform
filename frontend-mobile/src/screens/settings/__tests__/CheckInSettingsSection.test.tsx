// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Settings check-in choice — it reads and writes the server cadence and says when a save fails
// ABOUTME: The athlete API is mocked; assertions read the selected row and the calls (carnet#831)

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { CheckInSettingsSection } from '../CheckInSettingsSection';
import { athleteApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  athleteApi: {
    getCheckIn: jest.fn(),
    updateCheckIn: jest.fn(),
  },
}));

const getCheckIn = athleteApi.getCheckIn as jest.Mock;
const updateCheckIn = athleteApi.updateCheckIn as jest.Mock;

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <CheckInSettingsSection />
    </QueryClientProvider>,
  );
}

function selected(cadence: string) {
  return screen.getByTestId(`check-in-option-${cadence}`).props.accessibilityState;
}

describe('CheckInSettingsSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('offers the four cadences and marks the one stored', async () => {
    getCheckIn.mockResolvedValue({ cadence: 'weekly' });
    renderSection();

    await waitFor(() => expect(selected('weekly')).toMatchObject({ selected: true }));
    expect(screen.getByText('Every morning')).toBeTruthy();
    expect(screen.getByText('Monday, Wednesday and Friday')).toBeTruthy();
    expect(screen.getByText("Never — I'll start the conversation")).toBeTruthy();
    expect(selected('daily')).toMatchObject({ selected: false });
  });

  it('marks never for an athlete who never chose, which is what they get', async () => {
    getCheckIn.mockResolvedValue({ cadence: null });
    renderSection();

    await waitFor(() => expect(selected('off')).toMatchObject({ selected: true }));
  });

  it('stores a choice', async () => {
    getCheckIn.mockResolvedValue({ cadence: 'off' });
    updateCheckIn.mockResolvedValue({ cadence: 'daily' });
    renderSection();

    await waitFor(() => expect(selected('daily')).toMatchObject({ disabled: false }));
    fireEvent.press(screen.getByTestId('check-in-option-daily'));

    await waitFor(() => expect(updateCheckIn).toHaveBeenCalledWith('daily'));
    await waitFor(() => expect(selected('daily')).toMatchObject({ selected: true }));
  });

  it('says so when a choice could not be saved, and keeps what the server holds', async () => {
    getCheckIn.mockResolvedValue({ cadence: 'weekly' });
    updateCheckIn.mockRejectedValue(new Error('offline'));
    renderSection();

    await waitFor(() => expect(selected('daily')).toMatchObject({ disabled: false }));
    fireEvent.press(screen.getByTestId('check-in-option-daily'));

    expect(await screen.findByText('Your check-in choice could not be saved. Try again.')).toBeTruthy();
    await waitFor(() => expect(selected('weekly')).toMatchObject({ selected: true }));
  });
});
