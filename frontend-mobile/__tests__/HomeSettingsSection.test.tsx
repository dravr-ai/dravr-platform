// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings switch that brings back Home's plan suggestion on the phone — it reads and writes the server preference
// ABOUTME: Home's "Hide" turns the suggestion off; this switch is the way back, as on web (carnet#820)

import React from 'react';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockGetHomePreferences = jest.fn<Promise<{ plan_suggestion_hidden: boolean }>, []>();
const mockUpdateHomePreferences = jest.fn<
  Promise<{ plan_suggestion_hidden: boolean }>,
  [{ plan_suggestion_hidden: boolean }]
>();

jest.mock('../src/services/api', () => ({
  athleteApi: {
    getHomePreferences: () => mockGetHomePreferences(),
    updateHomePreferences: (prefs: { plan_suggestion_hidden: boolean }) => mockUpdateHomePreferences(prefs),
  },
}));

import { HomeSettingsSection } from '../src/screens/settings/HomeSettingsSection';

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <HomeSettingsSection />
    </QueryClientProvider>,
  );
}

describe('HomeSettingsSection', () => {
  beforeEach(() => jest.clearAllMocks());

  it('brings back a plan suggestion the athlete set aside', async () => {
    mockGetHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    mockUpdateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    const screen = renderSection();

    const toggle = screen.getByTestId('home-settings-plan-suggestion');
    await waitFor(() => expect(toggle.props.value).toBe(false));
    expect(toggle.props.accessibilityLabel).toBe('Suggest building a training plan');

    fireEvent(toggle, 'valueChange', true);
    await waitFor(() => expect(mockUpdateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: false }));
    await waitFor(() => expect(screen.getByTestId('home-settings-plan-suggestion').props.value).toBe(true));
  });

  it('sets the suggestion aside from Settings too', async () => {
    mockGetHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    mockUpdateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    const screen = renderSection();

    await waitFor(() => expect(screen.getByTestId('home-settings-plan-suggestion').props.disabled).toBe(false));
    fireEvent(screen.getByTestId('home-settings-plan-suggestion'), 'valueChange', false);
    await waitFor(() => expect(mockUpdateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true }));
  });
});
