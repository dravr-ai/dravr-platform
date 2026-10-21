// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the Settings switches that bring back Home's plan suggestion and Getting started — server preferences
// ABOUTME: Home's "Hide" links turn them off; these switches are the way back, each writing only its own choice (carnet#820, carnet#1098)

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import HomeSettings from '../HomeSettings';

type Prefs = { plan_suggestion_hidden: boolean; getting_started_hidden?: boolean };

const api = vi.hoisted(() => ({
  getHomePreferences: vi.fn<() => Promise<Prefs>>(),
  updateHomePreferences: vi.fn<(prefs: Partial<Prefs>) => Promise<Prefs>>(),
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
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true, getting_started_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: false });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Suggest building a training plan' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'false');

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: false });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });

  it('sets the suggestion aside from Settings too', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true, getting_started_hidden: false });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Suggest building a training plan' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ plan_suggestion_hidden: true });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
  });

  it('brings back a Getting started the athlete hid, leaving the plan choice alone', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true, getting_started_hidden: true });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: true, getting_started_hidden: false });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Getting started' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByText('Shown at the top of Today until your first steps are done.')).toBeInTheDocument();

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ getting_started_hidden: false });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
    expect(screen.getByRole('switch', { name: 'Suggest building a training plan' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
  });

  it('hides Getting started from Settings too', async () => {
    api.getHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: false });
    api.updateHomePreferences.mockResolvedValue({ plan_suggestion_hidden: false, getting_started_hidden: true });
    renderSettings();

    const toggle = screen.getByRole('switch', { name: 'Getting started' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toHaveAttribute('aria-checked', 'true');

    await userEvent.click(toggle);
    expect(api.updateHomePreferences).toHaveBeenCalledWith({ getting_started_hidden: true });
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'false'));
  });
});
