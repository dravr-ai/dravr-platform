// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile notification preferences screen
// ABOUTME: Asserts the switch sends the real category, the real value, and the rest of the row

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { NotificationPreferenceItem } from '@pierre/shared-types';
import { NotificationPreferencesScreen } from '../NotificationPreferencesScreen';
import { notificationsApi } from '../../../services/api';

jest.mock('expo-router', () => ({
  useRouter: () => ({ back: jest.fn(), push: jest.fn() }),
}));
jest.mock('../../../services/api', () => ({
  notificationsApi: {
    getPreferences: jest.fn(),
    updatePreference: jest.fn(),
  },
}));

const getPreferences = notificationsApi.getPreferences as jest.Mock;
const updatePreference = notificationsApi.updatePreference as jest.Mock;

/** A category row as the server returns it, with quiet hours and a cap set. */
function pref(overrides: Partial<NotificationPreferenceItem> = {}): NotificationPreferenceItem {
  return {
    category: 'training',
    enabled: true,
    sub_preferences: null,
    quiet_hours_start: '22:00',
    quiet_hours_end: '07:00',
    timezone: 'America/Toronto',
    max_per_day: 5,
    ...overrides,
  };
}

function renderScreen() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <NotificationPreferencesScreen />
    </QueryClientProvider>,
  );
}

describe('NotificationPreferencesScreen', () => {
  beforeEach(() => {
    getPreferences.mockReset();
    updatePreference.mockReset();
    updatePreference.mockResolvedValue(pref());
  });

  // Turns red if the screen stops rendering the server's rows — the exact
  // "hook complete, no screen" gap this surface closes.
  it('renders one row per category the server returned, labelled from shared metadata', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref(), pref({ category: 'coach', enabled: false })],
    });

    renderScreen();

    await waitFor(() => expect(screen.getByTestId('notification-pref-training')).toBeTruthy());
    expect(screen.getByTestId('notification-pref-coach')).toBeTruthy();
    expect(screen.getByText('Training')).toBeTruthy();
    expect(screen.getByText('Coach')).toBeTruthy();
    expect(screen.getByTestId('notification-pref-switch-training').props.value).toBe(true);
    expect(screen.getByTestId('notification-pref-switch-coach').props.value).toBe(false);
  });

  // Turns red if the switch sends the wrong category, the pre-toggle value, or
  // a partial row. The endpoint is an upsert: a request missing quiet_hours_end
  // writes NULL over it, so the athlete's quiet hours vanish on an unrelated mute.
  it('sends the real category and the flipped value, restating the rest of the row', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref({ category: 'coach', enabled: true })],
    });

    renderScreen();
    await waitFor(() => expect(screen.getByTestId('notification-pref-switch-coach')).toBeTruthy());

    fireEvent(screen.getByTestId('notification-pref-switch-coach'), 'valueChange', false);

    await waitFor(() => expect(updatePreference).toHaveBeenCalledTimes(1));
    expect(updatePreference).toHaveBeenCalledWith({
      category: 'coach',
      enabled: false,
      quiet_hours_start: '22:00',
      quiet_hours_end: '07:00',
      timezone: 'America/Toronto',
      max_per_day: 5,
    });
  });

  // Turns red if "No limit" starts sending max_per_day: 0 (a real cap of zero,
  // which mutes the category) instead of omitting the field so the upsert nulls it.
  it('clears the daily cap by omitting max_per_day rather than sending zero', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref({ category: 'recovery', max_per_day: 3 })],
    });

    renderScreen();
    await waitFor(() => expect(screen.getByTestId('notification-pref-details-recovery')).toBeTruthy());
    fireEvent.press(screen.getByTestId('notification-pref-details-recovery'));

    fireEvent.press(screen.getByTestId('notification-pref-cap-recovery-none'));

    await waitFor(() => expect(updatePreference).toHaveBeenCalledTimes(1));
    const sent = updatePreference.mock.calls[0][0] as Record<string, unknown>;
    expect(sent.category).toBe('recovery');
    expect('max_per_day' in sent).toBe(false);
    expect(sent.quiet_hours_start).toBe('22:00');
  });

  // Turns red if a quiet-hours change stops carrying a timezone — the server
  // compares HH:MM against the stored zone, so a null zone makes quiet hours
  // silently mean UTC.
  it('sends the chosen quiet-hours boundary with a timezone', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref({ category: 'ai', quiet_hours_start: null, timezone: 'Europe/Paris' })],
    });

    renderScreen();
    await waitFor(() => expect(screen.getByTestId('notification-pref-details-ai')).toBeTruthy());
    fireEvent.press(screen.getByTestId('notification-pref-details-ai'));

    fireEvent.press(screen.getByTestId('notification-pref-quiet-start-ai-23:00'));

    await waitFor(() => expect(updatePreference).toHaveBeenCalledTimes(1));
    expect(updatePreference.mock.calls[0][0]).toMatchObject({
      category: 'ai',
      quiet_hours_start: '23:00',
      timezone: 'Europe/Paris',
    });
  });
});
