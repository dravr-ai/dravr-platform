// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Settings units choice — it reads and writes the server preference and says what Automatic follows
// ABOUTME: The athlete API is mocked; assertions read the selected row, the calls and the hint (carnet#835)

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { UnitPreferences } from '@pierre/shared-types';
import { UnitsSettingsSection } from '../UnitsSettingsSection';
import { athleteApi } from '../../../services/api';

jest.mock('../../../services/api', () => ({
  athleteApi: {
    getUnitPreferences: jest.fn(),
    updateUnitPreferences: jest.fn(),
  },
}));

const getUnitPreferences = athleteApi.getUnitPreferences as jest.Mock;
const updateUnitPreferences = athleteApi.updateUnitPreferences as jest.Mock;

/** The device jest.setup.js reports through expo-localization. */
const DEVICE = 'en-US';

const FROM_STRAVA: UnitPreferences = {
  preference: 'automatic',
  units: 'imperial',
  source: 'provider',
  provider: 'strava',
  provider_units: 'imperial',
  device_locale: DEVICE,
};

function renderSection() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <UnitsSettingsSection />
    </QueryClientProvider>,
  );
}

describe('UnitsSettingsSection', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('selects Automatic and says it follows Strava, sending the device locale with the read', async () => {
    getUnitPreferences.mockResolvedValue(FROM_STRAVA);
    renderSection();

    expect(await screen.findByText('Automatic follows your Strava setting: imperial.')).toBeTruthy();
    expect(screen.getByTestId('units-option-automatic').props.accessibilityState).toMatchObject({ selected: true });
    expect(getUnitPreferences).toHaveBeenCalledWith(DEVICE);
    expect(updateUnitPreferences).not.toHaveBeenCalled();
  });

  it('stores an explicit choice, which needs no hint', async () => {
    getUnitPreferences.mockResolvedValue(FROM_STRAVA);
    updateUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, preference: 'metric', units: 'metric', source: 'override' });
    renderSection();

    await screen.findByTestId('units-settings-hint');
    await waitFor(() =>
      expect(screen.getByTestId('units-option-metric').props.accessibilityState).toMatchObject({ disabled: false }),
    );
    fireEvent.press(screen.getByTestId('units-option-metric'));

    await waitFor(() =>
      expect(updateUnitPreferences).toHaveBeenCalledWith({ preference: 'metric', device_locale: DEVICE }),
    );
    await waitFor(() =>
      expect(screen.getByTestId('units-option-metric').props.accessibilityState).toMatchObject({ selected: true }),
    );
    expect(screen.queryByTestId('units-settings-hint')).toBeNull();
  });

  it('reports the phone locale once when the server holds another', async () => {
    getUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, units: 'metric', source: 'locale', provider: null, provider_units: null, device_locale: null });
    updateUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, source: 'locale', provider: null, provider_units: null });
    renderSection();

    await waitFor(() =>
      expect(updateUnitPreferences).toHaveBeenCalledWith({ device_locale: DEVICE }),
    );
    expect(await screen.findByText("Automatic follows your device's region: imperial.")).toBeTruthy();
    expect(updateUnitPreferences).toHaveBeenCalledTimes(1);
  });
});
