// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the notification preferences screen speaks the reader's language, not English
// ABOUTME: Every string here was a hardcoded literal while the product default locale is French

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react-native';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  useFocusEffect: () => undefined,
}));

const mockUseNotificationPreferences = jest.fn();
jest.mock('../src/hooks/useNotifications', () => ({
  useNotificationPreferences: () => mockUseNotificationPreferences(),
}));

import { NotificationPreferencesScreen } from '../src/screens/settings/NotificationPreferencesScreen';
import { i18n } from '@pierre/i18n';

/** One enabled row, expanded far enough to reach the cap and quiet-hour labels. */
const trainingRow = {
  category: 'training' as const,
  enabled: true,
  max_per_day: null,
  quiet_hours_start: null,
  quiet_hours_end: null,
  timezone: 'America/Montreal',
};

function loaded() {
  return {
    preferences: [trainingRow],
    isLoading: false,
    isError: false,
    updatePreference: jest.fn(),
    isUpdating: false,
  };
}

describe('NotificationPreferencesScreen in French', () => {
  // The suite pins English (jest.setup.js), so a test about the product's own
  // default locale has to ask for it, and hand it back afterwards.
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  beforeEach(async () => {
    await i18n.changeLanguage('fr');
    jest.clearAllMocks();
    mockUseNotificationPreferences.mockReturnValue(loaded());
  });

  it('renders the category blurb in French, not the English literal', async () => {
    render(<NotificationPreferencesScreen />);

    // The blurb was a hardcoded English sentence in a Record<category, string>.
    // Asserted on the real French copy rather than "not English", because an
    // empty render would also satisfy a negative assertion.
    const blurb = i18n.t('notifPrefs.blurbTraining');
    expect(blurb).not.toMatch(/Planned sessions/);
    await waitFor(() => {
      expect(screen.getByText(blurb)).toBeTruthy();
    });
  });

  it('renders the screen title and intro from the corpus', async () => {
    render(<NotificationPreferencesScreen />);

    await waitFor(() => {
      expect(screen.getByText(i18n.t('notifPrefs.title'))).toBeTruthy();
    });
    expect(screen.getByText(i18n.t('notifPrefs.intro'))).toBeTruthy();
  });

  it('tells a phone to go back, never to reload a page it does not have', async () => {
    mockUseNotificationPreferences.mockReturnValue({
      ...loaded(),
      isError: true,
      preferences: [],
    });
    render(<NotificationPreferencesScreen />);

    const mobileCopy = i18n.t('notifPrefs.loadFailedMobile');
    await waitFor(() => {
      expect(screen.getByTestId('notification-prefs-error')).toHaveTextContent(mobileCopy);
    });
    // The web twin's string says "Recharge la page". A phone has no page to
    // reload, which is why this screen carries its own key rather than sharing
    // that one.
    expect(mobileCopy).not.toEqual(i18n.t('notifPrefs.loadFailed'));
  });

  it('every locale carries the seven category blurbs the screen maps', async () => {
    // The screen maps category -> key; a key missing from a locale renders the
    // raw key string to the athlete. i18next returns the key itself on a miss,
    // so comparing against it catches exactly that.
    for (const locale of ['fr', 'en', 'es', 'de', 'pt']) {
      await i18n.changeLanguage(locale);
      for (const category of [
        'Training',
        'Recovery',
        'Coach',
        'Achievement',
        'System',
        'Ai',
        'Reminders',
      ]) {
        const key = `notifPrefs.blurb${category}`;
        expect(i18n.t(key)).not.toEqual(key);
      }
    }
  });
});
