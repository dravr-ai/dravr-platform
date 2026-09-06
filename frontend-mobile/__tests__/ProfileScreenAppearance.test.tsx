// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the Profile pane carries appearance and language, and that the tiles lay out evenly
// ABOUTME: Web groups both with the profile; the five language tiles used to wrap 4 + 1 and strand Portuguese

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
import { languageGridColumns, languageGridRows } from '../src/components/LanguageSwitcher';
import { i18n } from '@pierre/i18n';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

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

  it('holds appearance beside the profile, as web does', async () => {
    render(<ProfileScreen />);

    await waitFor(() => {
      expect(screen.getByTestId('profile-appearance-section')).toBeTruthy();
    });
    for (const option of ['system', 'dark', 'light']) {
      expect(screen.getByTestId(`appearance-option-${option}`)).toBeTruthy();
    }
  });

  it('mounts the switcher with all five locales, French selected', async () => {
    render(<ProfileScreen />);

    await waitFor(() => {
      expect(screen.getByTestId('profile-language-section')).toBeTruthy();
    });
    expect(screen.getByTestId('language-switcher')).toBeTruthy();
    for (const locale of ['fr', 'en', 'es', 'de', 'pt']) {
      expect(screen.getByTestId(`language-option-${locale}`)).toBeTruthy();
    }
    expect(screen.getByTestId('language-option-fr').props.accessibilityState.selected).toBe(true);
    expect(
      screen.getByText('L’interface et les réponses de ton agent suivent toutes deux ce réglage.'),
    ).toBeTruthy();
  });

  it('never leaves a single tile alone on the last line', () => {
    // The plain wrapping row laid the five locales out 4 + 1 on a phone, so
    // Portuguese sat by itself under four siblings. The column count is chosen
    // rather than inherited, and the rule has to hold for any locale count —
    // adding a sixth must not re-create the orphan.
    for (let count = 2; count <= 12; count += 1) {
      const columns = languageGridColumns(count);
      const rows = languageGridRows([...Array(count).keys()], columns);
      expect(rows[rows.length - 1].length).toBeGreaterThan(1);
      expect(rows.flat()).toHaveLength(count);
    }
  });

  it('lays the shipped locales out three to a line', () => {
    const columns = languageGridColumns(SUPPORTED_LANGUAGES.length);
    expect(SUPPORTED_LANGUAGES).toHaveLength(5);
    expect(columns).toBe(3);
    expect(languageGridRows(SUPPORTED_LANGUAGES, columns).map((row) => row.length)).toEqual([3, 2]);
  });

  it('renders one row element per computed line', async () => {
    render(<ProfileScreen />);
    await waitFor(() => {
      expect(screen.getByTestId('language-row-0')).toBeTruthy();
    });
    expect(screen.getByTestId('language-row-1')).toBeTruthy();
    expect(screen.queryByTestId('language-row-2')).toBeNull();
  });
});
