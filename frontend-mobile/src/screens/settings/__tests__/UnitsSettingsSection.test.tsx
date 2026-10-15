// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the Settings units choice — Metric or Imperial, read from and written to the server preference
// ABOUTME: The athlete API is mocked; assertions read the rows, the selected one and the calls (carnet#835, carnet#1044)

import React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
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

  it('offers Metric and Imperial only, selecting what the server holds', async () => {
    getUnitPreferences.mockResolvedValue({ units: 'metric' });
    renderSection();

    await waitFor(() =>
      expect(screen.getByTestId('units-option-metric').props.accessibilityState).toMatchObject({ selected: true }),
    );
    expect(screen.getByTestId('units-option-imperial').props.accessibilityState).toMatchObject({ selected: false });
    expect(screen.queryByTestId('units-option-automatic')).toBeNull();
    expect(getUnitPreferences).toHaveBeenCalledWith();
    expect(updateUnitPreferences).not.toHaveBeenCalled();
  });

  it('stores a choice', async () => {
    getUnitPreferences.mockResolvedValue({ units: 'metric' });
    updateUnitPreferences.mockResolvedValue({ units: 'imperial' });
    renderSection();

    await waitFor(() =>
      expect(screen.getByTestId('units-option-imperial').props.accessibilityState).toMatchObject({ disabled: false }),
    );
    fireEvent.press(screen.getByTestId('units-option-imperial'));

    await waitFor(() => expect(updateUnitPreferences).toHaveBeenCalledWith('imperial'));
    await waitFor(() =>
      expect(screen.getByTestId('units-option-imperial').props.accessibilityState).toMatchObject({ selected: true }),
    );
  });
});
