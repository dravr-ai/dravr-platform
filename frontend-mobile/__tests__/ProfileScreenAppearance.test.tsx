// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the Profile pane carries appearance and language, each as a radio list of rows with one selected
// ABOUTME: Web groups both with the profile; the five locales are rows named in their own language, never flag tiles

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react-native';
import type { User } from '@pierre/shared-types';
import { SUPPORTED_LANGUAGES } from '@pierre/i18n';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  useFocusEffect: () => undefined,
}));

jest.mock('../src/services/api', () => ({
  userApi: {
    updateProfile: jest.fn(),
    updateTheme: jest.fn().mockResolvedValue({}),
  },
}));

const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { ProfileScreen } from '../src/screens/settings/ProfileScreen';
import { i18n } from '@pierre/i18n';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

/** The ids of the rendered rows whose accessibility state says selected. */
function selectedRows(ids: readonly string[]): string[] {
  return ids.filter((id) => screen.getByTestId(id).props.accessibilityState?.selected === true);
}

describe('Profile pane — appearance and language', () => {
  // This asserts French is the preselected locale, which is the product
  // default; the suite pins English, so it selects French explicitly.
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  beforeEach(async () => {
    await i18n.changeLanguage('fr');
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      user: baseUser as User,
      updateUser: jest.fn(),
      logout: jest.fn(),
      isAuthenticated: true,
    });
  });

  it('holds appearance beside the profile, as web does, as a radio list with one choice in force', async () => {
    render(<ProfileScreen />);

    await waitFor(() => {
      expect(screen.getByTestId('profile-appearance-section')).toBeTruthy();
    });
    const ids = ['system', 'dark', 'light'].map((option) => `appearance-option-${option}`);
    for (const id of ids) {
      expect(screen.getByTestId(id).props.accessibilityRole).toBe('radio');
    }
    expect(selectedRows(ids)).toHaveLength(1);
  });

  it('mounts the switcher with all five locales as rows, French selected', async () => {
    render(<ProfileScreen />);

    await waitFor(() => {
      expect(screen.getByTestId('profile-language-section')).toBeTruthy();
    });
    expect(screen.getByTestId('language-switcher')).toBeTruthy();
    expect(SUPPORTED_LANGUAGES).toHaveLength(5);
    const ids = ['fr', 'en', 'es', 'de', 'pt'].map((locale) => `language-option-${locale}`);
    for (const id of ids) {
      expect(screen.getByTestId(id).props.accessibilityRole).toBe('radio');
    }
    expect(selectedRows(ids)).toEqual(['language-option-fr']);
    expect(
      screen.getByText('L’interface et les réponses de ton agent suivent toutes deux ce réglage.'),
    ).toBeTruthy();
  });

  it('names each locale in its own language, with no flag in front of it', async () => {
    // The tiles carried a flag emoji above the name; a flag is a country, not
    // a language, and the row grammar has no glyph slot. The name alone is the
    // row's title.
    render(<ProfileScreen />);
    await waitFor(() => {
      expect(screen.getByTestId('language-option-pt')).toBeTruthy();
    });
    expect(screen.getByText('Português')).toBeTruthy();
    expect(screen.getByText('Deutsch')).toBeTruthy();
    expect(screen.queryByText(/🇫🇷|🇬🇧|🇪🇸|🇩🇪|🇵🇹/u)).toBeNull();
    expect(screen.queryByTestId('language-row-0')).toBeNull();
  });
});
