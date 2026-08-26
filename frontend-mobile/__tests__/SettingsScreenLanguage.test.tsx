// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the mobile Settings screen actually mounts the language switcher
// ABOUTME: The switcher shipped on both clients and was reachable from neither — this fails if it goes back

import React from 'react';
import { render, screen, waitFor } from '@testing-library/react-native';
import type { User } from '@pierre/shared-types';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  useFocusEffect: () => undefined,
}));

jest.mock('../src/services/api', () => ({
  userApi: {
    getMcpTokens: jest.fn().mockResolvedValue({ tokens: [] }),
    createMcpToken: jest.fn(),
    changePassword: jest.fn(),
  },
  oauthApi: {
    getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }),
  },
}));

jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({ data: null, isLoading: false }),
}));

const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: false, billing_header: false }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

describe('SettingsScreen language section', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      user: baseUser as User,
      logout: jest.fn(),
      isAuthenticated: true,
    });
  });

  it('mounts the switcher with all five locales, French selected', async () => {
    render(<SettingsScreen />);

    await waitFor(() => {
      expect(screen.getByTestId('settings-language-section')).toBeTruthy();
    });
    expect(screen.getByTestId('language-switcher')).toBeTruthy();
    for (const locale of ['fr', 'en', 'es', 'de', 'pt']) {
      expect(screen.getByTestId(`language-option-${locale}`)).toBeTruthy();
    }
    expect(screen.getByTestId('language-option-fr').props.accessibilityState.selected).toBe(true);
    expect(
      screen.getByText("L'interface et les réponses de ton coach suivent toutes deux ce réglage."),
    ).toBeTruthy();
  });
});
