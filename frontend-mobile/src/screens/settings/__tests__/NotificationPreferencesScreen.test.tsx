// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile notification preferences screen
// ABOUTME: Asserts the switch sends the real category, the real value, and the rest of the row

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { NOTIFICATION_CATEGORIES } from '@pierre/shared-constants';
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

type Json = { type: string; props: Record<string, unknown>; children: Array<Json | string> | null };

/** Every style object in a rendered tree, flat or nested arrays alike. */
function allStyles(node: Json | string | null | undefined, out: Array<Record<string, unknown>> = []) {
  if (!node || typeof node === 'string') return out;
  const flatten = (style: unknown): void => {
    if (Array.isArray(style)) style.forEach(flatten);
    else if (style && typeof style === 'object') out.push(style as Record<string, unknown>);
  };
  flatten(node.props.style);
  for (const child of node.children ?? []) allStyles(child, out);
  return out;
}

/** The rendered tree as one node, whichever shape `toJSON()` returned. */
function rootOf(tree: ReturnType<typeof render>['toJSON']): Json {
  const rendered = tree() as Json | Json[] | null;
  if (!rendered) throw new Error('nothing rendered');
  return Array.isArray(rendered) ? rendered[0] : rendered;
}

describe('NotificationPreferencesScreen', () => {
  beforeEach(() => {
    getPreferences.mockReset();
    updatePreference.mockReset();
    updatePreference.mockResolvedValue(pref());
  });

  // Turns red if an account with no stored override renders nothing. The
  // endpoint returns overrides, so every athlete starts with `preferences: []`
  // and the screen kept only the categories that came back — which was none of
  // them, and the pane painted its heading over an empty page.
  it('renders every category at its default when the server stored no override', async () => {
    getPreferences.mockResolvedValue({ user_id: 'u1', tenant_id: 't1', preferences: [] });

    renderScreen();

    await waitFor(() => expect(screen.getByTestId('notification-pref-training')).toBeTruthy());
    for (const category of NOTIFICATION_CATEGORIES) {
      expect(screen.getByTestId(`notification-pref-${category}`)).toBeTruthy();
      expect(screen.getByTestId(`notification-pref-switch-${category}`).props.value).toBe(true);
    }
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
    expect(screen.getByText('Agent')).toBeTruthy();
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

  // Turns red if the card comes back: the seven categories are seven sections
  // in a gap-8 column, with no colour dot before the label and no pill anywhere
  // (Boreal v2.2, DESIGN.md §10 — sections are separated by space, not a box).
  it('lays each category out as its own section, with no card, colour dot or pill', async () => {
    getPreferences.mockResolvedValue({ user_id: 'u1', tenant_id: 't1', preferences: [pref()] });

    const { toJSON } = renderScreen();
    await waitFor(() => expect(screen.getByTestId('notification-pref-training')).toBeTruthy());

    expect(screen.getAllByTestId(/^notification-pref-(training|recovery|coach|achievement|system|ai|reminders)$/)).toHaveLength(7);
    expect(screen.getByTestId('notification-prefs-list').props.className).toContain('gap-8');
    for (const category of NOTIFICATION_CATEGORIES) {
      const section = screen.getByTestId(`notification-pref-${category}`);
      expect(section.props.className).not.toContain('bg-');
      expect(section.props.className).not.toContain('border');
      expect(section.props.style).toBeUndefined();
    }

    const serialised = JSON.stringify(toJSON());
    expect(serialised).not.toContain('rounded-full');
    expect(serialised).not.toContain('w-2.5');
    const styles = allStyles(rootOf(toJSON));
    expect(styles.some((s) => typeof s.borderRadius === 'number' && s.borderRadius >= 999)).toBe(false);
    expect(styles.some((s) => s.width === 10 && s.height === 10)).toBe(false);
  });

  // Turns red if the cap picker stops being text tabs: the stored cap is the
  // one selected tab, "no limit" is a tab too, and the tabs say so to a screen
  // reader rather than being anonymous chips.
  it('offers the daily cap as text tabs with the stored cap selected', async () => {
    getPreferences.mockResolvedValue({
      user_id: 'u1',
      tenant_id: 't1',
      preferences: [pref({ category: 'recovery', max_per_day: 3 })],
    });

    renderScreen();
    await waitFor(() => expect(screen.getByTestId('notification-pref-details-recovery')).toBeTruthy());
    fireEvent.press(screen.getByTestId('notification-pref-details-recovery'));

    const stored = screen.getByTestId('notification-pref-cap-recovery-3');
    expect(stored.props.accessibilityRole).toBe('tab');
    expect(stored.props.accessibilityState.selected).toBe(true);
    const none = screen.getByTestId('notification-pref-cap-recovery-none');
    expect(none.props.accessibilityRole).toBe('tab');
    expect(none.props.accessibilityState.selected).toBe(false);
    expect(screen.getByTestId('notification-pref-quiet-start-recovery-22:00').props.accessibilityState.selected).toBe(true);
    expect(screen.getByTestId('notification-pref-quiet-end-recovery-off').props.accessibilityState.selected).toBe(false);
  });
});
