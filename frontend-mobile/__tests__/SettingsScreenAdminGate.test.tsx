// ABOUTME: Regression tests for the settings list rows by account role
// ABOUTME: The phone has no admin console, so an admin gets every athlete pane a regular user gets (carnet#1631)

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';

// Mock Feather icons
jest.mock('@expo/vector-icons', () => ({
  Feather: () => null,
}));

// Role is switchable per test: jest evaluates the factory's returned function
// lazily on each useAuth() call, so reassigning mockRole changes the user's role.
let mockRole: 'user' | 'admin' = 'user';

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

describe('SettingsScreen - every role gets the athlete panes', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it.each(['user', 'admin'] as const)('shows the athlete-account panes to a %s', async (role) => {
    mockRole = role;
    const { getByTestId } = render(<SettingsScreen />);

    await waitFor(() => {
      expect(getByTestId('settings-pane-list')).toBeTruthy();
    });
    // The panes the web admin console hides: the phone shows every one of them.
    for (const id of ADMIN_HIDDEN_PANES) {
      expect(getByTestId(`settings-pane-${id}`)).toBeTruthy();
    }
    expect(getByTestId('settings-pane-account')).toBeTruthy();
  });
});
