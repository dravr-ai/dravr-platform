// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings switch that brings back Home's plan suggestion — it reads and writes the server preference
// ABOUTME: Home's "Hide" turns the suggestion off; this switch is the way back (carnet#820)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import HomeSettings from '../HomeSettings';

const api = vi.hoisted(() => ({
  getHomePreferences: vi.fn<() => Promise<{ plan_suggestion_hidden: boolean }>>(),
  updateHomePreferences:
    vi.fn<(prefs: { plan_suggestion_hidden: boolean }) => Promise<{ plan_suggestion_hidden: boolean }>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: {
    getHomePreferences: api.getHomePreferences,
    updateHomePreferences: api.updateHomePreferences,
  },
}));

function renderSettings() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={client}>
      <HomeSettings />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('HomeSettings', () => {
  it('brings back a plan suggestion the athlete set aside', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Suggest building a training plan' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: false });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  it('sets the suggestion aside from Settings too', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Suggest building a training plan' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
  });
});
