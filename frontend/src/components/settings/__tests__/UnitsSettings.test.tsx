// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings units choice — it reads and writes the server preference and says what Automatic resolved to
// ABOUTME: Red if the choice is not stored, the device locale is never reported, or a refused save goes unsaid (carnet#835)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { UnitPreferences, UnitPreferencesUpdate } from '@pierre/shared-types';
import UnitsSettings from '../UnitsSettings';

const api = vi.hoisted(() => ({
  getUnitPreferences: vi.fn<(deviceLocale: string) => Promise<UnitPreferences>>(),
  updateUnitPreferences: vi.fn<(update: UnitPreferencesUpdate) => Promise<UnitPreferences>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getUnitPreferences: api.getUnitPreferences,
    updateUnitPreferences: api.updateUnitPreferences,
  },
}));

const DEVICE = navigator.language;

const FROM_STRAVA: UnitPreferences = {
  preference: 'automatic',
  units: 'imperial',
  source: 'provider',
  provider: 'strava',
  provider_units: 'imperial',
  device_locale: DEVICE,
};

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <UnitsSettings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('UnitsSettings', () => {
  it('says what Automatic follows, and sends the device locale with the read', async () => {
    api.getUnitPreferences.mockResolvedValue(FROM_STRAVA);
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    expect(select).toHaveValue('automatic');
    expect(screen.getByTestId('units-settings-hint')).toHaveTextContent(
      'Automatic follows your Strava setting: imperial.',
    );
    expect(api.getUnitPreferences).toHaveBeenCalledWith(DEVICE);
    // The stored locale already matches this device: nothing to write.
    expect(api.updateUnitPreferences).not.toHaveBeenCalled();
  });

  it('stores an explicit choice, which needs no hint', async () => {
    api.getUnitPreferences.mockResolvedValue(FROM_STRAVA);
    api.updateUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, preference: 'metric', units: 'metric', source: 'override' });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'metric');

    expect(api.updateUnitPreferences).toHaveBeenCalledWith({ preference: 'metric', device_locale: DEVICE });
    await waitFor(() => expect(select).toHaveValue('metric'));
    expect(screen.queryByTestId('units-settings-hint')).not.toBeInTheDocument();
  });

  it('reports this device locale once when the server holds another', async () => {
    api.getUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, source: 'locale', provider: null, provider_units: null, units: 'metric', device_locale: null });
    api.updateUnitPreferences.mockResolvedValue({ ...FROM_STRAVA, source: 'locale', provider: null, provider_units: null });
    renderSettings();

    await waitFor(() =>
      expect(api.updateUnitPreferences).toHaveBeenCalledWith({ device_locale: DEVICE }),
    );
    expect(api.updateUnitPreferences).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(screen.getByTestId('units-settings-hint')).toHaveTextContent(
        "Automatic follows your device's region: imperial.",
      ),
    );
  });

  it('says so when a choice could not be saved, and keeps what the server holds', async () => {
    api.getUnitPreferences.mockResolvedValue(FROM_STRAVA);
    api.updateUnitPreferences.mockRejectedValue(new Error('offline'));
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'metric');

    expect(await screen.findByRole('alert')).toHaveTextContent('Your units could not be saved. Try again.');
    await waitFor(() => expect(select).toHaveValue('automatic'));
  });
});
