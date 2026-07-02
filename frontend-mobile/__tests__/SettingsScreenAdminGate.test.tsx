// ABOUTME: Regression tests for pure-operator gating of the Data Providers section in SettingsScreen
// ABOUTME: Admins are platform operators and must not see personal provider surfaces; regular users must

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';

// Mock safe area
jest.mock('react-native-safe-area-context', () => ({
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));

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

// Mock shared-constants
jest.mock('@pierre/shared-constants', () => ({
  QUERY_KEYS: { usage: { status: () => ['usage', 'status'] } },
}));

// Mock useUsageStatus with minimal quota data
jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: {
      daily: {
        messages: { allowed: true, current: 1, limit: 100, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tokens: { allowed: true, current: 1000, limit: 500000, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tool_calls: { allowed: true, current: 1, limit: 50, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
      },
      weekly: {
        messages: { allowed: true, current: 1, limit: 500, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tokens: { allowed: true, current: 1000, limit: 2000000, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tool_calls: { allowed: true, current: 1, limit: 200, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
      },
      resources: {
        conversations: 1,
        max_conversations: 10,
        coaches: 1,
        max_coaches: 3,
      },
    },
    isLoading: false,
    level: 'none',
    sendDisabled: false,
    message: '',
    resetsAt: '',
    invalidate: jest.fn(),
  }),
}));

// Must import AFTER mocks
import { SettingsScreen } from '../src/screens/settings/SettingsScreen';

describe('SettingsScreen - admin pure-operator gating', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('shows the Data Providers section for regular users', async () => {
    mockRole = 'user';
    const { getByTestId, getByText } = render(<SettingsScreen />);

    await waitFor(() => {
      expect(getByTestId('settings-data-section')).toBeTruthy();
      expect(getByText('Data Providers')).toBeTruthy();
    });
  });

  it('hides the Data Providers section for admins', async () => {
    mockRole = 'admin';
    const { queryByTestId, queryByText, getByTestId } = render(<SettingsScreen />);

    // Wait for the screen to settle on a section that renders for everyone.
    await waitFor(() => {
      expect(getByTestId('settings-account-section')).toBeTruthy();
    });
    expect(queryByTestId('settings-data-section')).toBeNull();
    expect(queryByText('Data Providers')).toBeNull();
  });

  it('hides the Data Providers section for super admins', async () => {
    mockRole = 'super_admin';
    const { queryByTestId, getByTestId } = render(<SettingsScreen />);

    await waitFor(() => {
      expect(getByTestId('settings-account-section')).toBeTruthy();
    });
    expect(queryByTestId('settings-data-section')).toBeNull();
  });
});
