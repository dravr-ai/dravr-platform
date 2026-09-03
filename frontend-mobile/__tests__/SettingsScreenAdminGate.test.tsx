// ABOUTME: Regression tests for pure-operator gating of the settings list rows
// ABOUTME: Admins are platform operators and must not see personal athlete panes; regular users must

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';


// Mock LinearGradient
jest.mock('expo-linear-gradient', () => ({
  LinearGradient: ({ children, ...props }: { children: React.ReactNode }) => {
    const { View } = require('react-native');
    return <View {...props}>{children}</View>;
  },
}));

// Mock Feather icons
jest.mock('@expo/vector-icons', () => ({
  Feather: () => null,
}));

// Role is switchable per test: jest evaluates the factory's returned function
// lazily on each useAuth() call, so reassigning mockRole changes the user's role.
let mockRole: 'user' | 'admin' | 'super_admin' = 'user';

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'test@pierre.dev',
      display_name: 'Test User',
      role: mockRole,
    },
    logout: jest.fn(),
    isAuthenticated: true,
  }),
}));

// Mock API service
jest.mock('../src/services/api', () => ({
  userApi: {
    getMcpTokens: jest.fn().mockResolvedValue({ tokens: [] }),
  },
  oauthApi: {
    getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }),
  },
  apiClient: { get: jest.fn() },
}));

// Must import AFTER mocks
// The API Tokens pane is gated on the shared `api_tokens` flag. These specs are
// about the rest of the list, so the flag hook answers with its off default
// rather than dragging a QueryClientProvider into every render.
jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: false, billing_header: false }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';
import { ADMIN_HIDDEN_PANES } from '@pierre/shared-constants';

describe('SettingsScreen - admin pure-operator gating', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('shows the athlete-account panes for regular users', async () => {
    mockRole = 'user';
    const { getByTestId } = render(<SettingsScreen />);

    await waitFor(() => {
      expect(getByTestId('settings-pane-list')).toBeTruthy();
    });
    for (const id of ADMIN_HIDDEN_PANES) {
      expect(getByTestId(`settings-pane-${id}`)).toBeTruthy();
    }
  });

  it.each(['admin', 'super_admin'] as const)('hides them for %s', async (role) => {
    mockRole = role;
    const { queryByTestId, getByTestId } = render(<SettingsScreen />);

    // Wait for the list itself, which renders for everyone.
    await waitFor(() => {
      expect(getByTestId('settings-pane-list')).toBeTruthy();
    });
    for (const id of ADMIN_HIDDEN_PANES) {
      expect(queryByTestId(`settings-pane-${id}`)).toBeNull();
    }
    // The Account pane is not an athlete surface and stays.
    expect(getByTestId('settings-pane-account')).toBeTruthy();
  });
});
