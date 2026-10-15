// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings units choice — Metric or Imperial, read from and written to the server preference
// ABOUTME: Red if Automatic comes back, the choice is not stored, or a refused save goes unsaid (carnet#835, carnet#1044)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { UnitPreferences, UnitSystem } from '@pierre/shared-types';
import UnitsSettings from '../UnitsSettings';

const api = vi.hoisted(() => ({
  getUnitPreferences: vi.fn<() => Promise<UnitPreferences>>(),
  updateUnitPreferences: vi.fn<(units: UnitSystem) => Promise<UnitPreferences>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getUnitPreferences: api.getUnitPreferences,
    updateUnitPreferences: api.updateUnitPreferences,
  },
}));

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
  it('offers Metric and Imperial only, showing what the server holds', async () => {
    api.getUnitPreferences.mockResolvedValue({ units: 'metric' });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    expect(select).toHaveValue('metric');
    expect(within(select).getAllByRole('option').map((option) => option.getAttribute('value'))).toEqual([
      'metric',
      'imperial',
    ]);
    expect(api.getUnitPreferences).toHaveBeenCalledWith();
    expect(api.updateUnitPreferences).not.toHaveBeenCalled();
  });

  it('stores a choice', async () => {
    api.getUnitPreferences.mockResolvedValue({ units: 'metric' });
    api.updateUnitPreferences.mockResolvedValue({ units: 'imperial' });
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'imperial');

    expect(api.updateUnitPreferences).toHaveBeenCalledWith('imperial');
    await waitFor(() => expect(select).toHaveValue('imperial'));
  });

  it('says so when a choice could not be saved, and keeps what the server holds', async () => {
    api.getUnitPreferences.mockResolvedValue({ units: 'metric' });
    api.updateUnitPreferences.mockRejectedValue(new Error('offline'));
    renderSettings();

    const select = screen.getByRole('combobox', { name: 'Select units' });
    await waitFor(() => expect(select).toBeEnabled());
    await userEvent.selectOptions(select, 'imperial');

    expect(await screen.findByRole('alert')).toHaveTextContent('Your units could not be saved. Try again.');
    await waitFor(() => expect(select).toHaveValue('metric'));
  });
});
