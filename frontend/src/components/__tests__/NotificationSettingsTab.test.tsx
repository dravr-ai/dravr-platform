// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the web notification preferences tab
// ABOUTME: Asserts the switch sends the real category, the real value, and the rest of the row

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NOTIFICATION_CATEGORIES } from '@pierre/shared-constants';
import type { NotificationPreferenceItem } from '@pierre/shared-types';
import NotificationSettingsTab from '../NotificationSettingsTab';

const getPreferences = vi.fn();
const updatePreference = vi.fn();

vi.mock('../../services/api', () => ({
  notificationsApi: {
    getPreferences: (...args: unknown[]) => getPreferences(...args),
    updatePreference: (...args: unknown[]) => updatePreference(...args),
  },
}));

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

function renderTab() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <NotificationSettingsTab />
    </QueryClientProvider>,
  );
}

describe('NotificationSettingsTab', () => {
  beforeEach(() => {
    getPreferences.mockReset();
    updatePreference.mockReset();
    updatePreference.mockResolvedValue(pref());
  });

  // Turns red if an account with no stored override renders nothing. The
  // endpoint returns overrides, so every athlete starts with `preferences: []`
  // and the tab kept only the categories that came back — which was none of
  // them, and the pane painted its heading over an empty page.
  it('renders every category at its default when the server stored no override', async () => {
    getPreferences.mockResolvedValue({ user_id: 'u1', tenant_id: 't1', preferences: [] });

    renderTab();

    await waitFor(() => expect(screen.getByTestId('notification-pref-training')).toBeTruthy());
    for (const category of NOTIFICATION_CATEGORIES) {
      expect(screen.getByTestId(`notification-pref-${category}`)).toBeTruthy();
      expect(
        screen.getByTestId(`notification-pref-switch-${category}`).getAttribute('aria-checked'),
      ).toBe('true');
    }
  });

  // Turns red if the tab stops rendering the server's rows — the exact
  // "backend exists, nothing renders it" gap this surface closes.
  it('renders one row per category the server returned, labelled from shared metadata', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref(), pref({ category: 'coach', enabled: false })],
    });

    renderTab();

    await waitFor(() => expect(screen.getByTestId('notification-pref-training')).toBeTruthy());
    expect(screen.getByTestId('notification-pref-coach')).toBeTruthy();
    expect(screen.getByText('Training')).toBeTruthy();
    expect(screen.getByText('Coach')).toBeTruthy();
    expect(screen.getByTestId('notification-pref-switch-training').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('notification-pref-switch-coach').getAttribute('aria-checked')).toBe('false');
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

    renderTab();
    await waitFor(() => expect(screen.getByTestId('notification-pref-switch-coach')).toBeTruthy());

    await userEvent.click(screen.getByTestId('notification-pref-switch-coach'));

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

    renderTab();
    await waitFor(() => expect(screen.getByTestId('notification-pref-details-recovery')).toBeTruthy());
    await userEvent.click(screen.getByTestId('notification-pref-details-recovery'));

    const cap = screen.getByTestId('notification-pref-cap-recovery') as HTMLSelectElement;
    await userEvent.selectOptions(cap, '');

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

    renderTab();
    await waitFor(() => expect(screen.getByTestId('notification-pref-details-ai')).toBeTruthy());
    await userEvent.click(screen.getByTestId('notification-pref-details-ai'));

    await userEvent.selectOptions(
      screen.getByTestId('notification-pref-quiet-start-ai') as HTMLSelectElement,
      '23:00',
    );

    await waitFor(() => expect(updatePreference).toHaveBeenCalledTimes(1));
    expect(updatePreference.mock.calls[0][0]).toMatchObject({
      category: 'ai',
      quiet_hours_start: '23:00',
      timezone: 'Europe/Paris',
    });
  });
});
