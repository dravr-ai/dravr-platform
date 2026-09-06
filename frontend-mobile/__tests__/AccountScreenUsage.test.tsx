// ABOUTME: Unit tests for the Account pane — its section grouping and the quota meters inside it
// ABOUTME: Usage and connected MCP apps stood alone on the phone while web held them under Account

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { settingsPaneSections } from '@pierre/shared-constants';


jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
}));

// Mock Feather icons
jest.mock('@expo/vector-icons', () => ({
  Feather: () => null,
}));

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'test@pierre.dev',
      display_name: 'Test User',
      role: 'user',
      user_status: 'active',
      created_at: '2026-01-15T12:00:00Z',
    },
    logout: jest.fn(),
    isAuthenticated: true,
  }),
}));

// Mock API service
jest.mock('../src/services/api', () => ({
  userApi: {
    changePassword: jest.fn(),
  },
  apiClient: { get: jest.fn() },
}));

// Mock useUsageStatus with realistic quota data
jest.mock('../src/screens/chat/useUsageStatus', () => ({
  useUsageStatus: () => ({
    data: {
      daily: {
        messages: { allowed: true, current: 42, limit: 100, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tokens: { allowed: true, current: 145000, limit: 500000, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
        tool_calls: { allowed: true, current: 10, limit: 50, warning: false, burst_zone: false, resets_at: '2026-02-19T05:00:00Z' },
      },
      weekly: {
        messages: { allowed: true, current: 200, limit: 500, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tokens: { allowed: true, current: 900000, limit: 2000000, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
        tool_calls: { allowed: true, current: 40, limit: 200, warning: false, burst_zone: false, resets_at: '2026-02-24T05:00:00Z' },
      },
      resources: {
        conversations: 5,
        max_conversations: 10,
        coaches: 2,
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
import { AccountScreen } from '../src/screens/settings/AccountScreen';

describe('AccountScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('renders every section the shared declaration groups under Account', async () => {
    // Web has held status, usage, security and the connected MCP apps together
    // since it had panes; the phone scattered them down one scroll.
    const { getByTestId } = render(<AccountScreen />);

    await waitFor(() => {
      expect(getByTestId('account-screen')).toBeTruthy();
    });
    const sections = settingsPaneSections('account');
    expect([...sections]).toEqual([
      'account-status',
      'usage',
      'security',
      'connected-mcp-apps',
      'sign-out',
    ]);
    for (const section of sections) {
      expect(getByTestId(`account-section-${section}`)).toBeTruthy();
    }
  });

  it('should render usage section with quota meter labels', async () => {
    const { getByText } = render(<AccountScreen />);

    await waitFor(() => {
      expect(getByText('Usage')).toBeTruthy();
      expect(getByText('Daily Messages')).toBeTruthy();
      expect(getByText('Daily Tokens')).toBeTruthy();
      expect(getByText('Weekly Messages')).toBeTruthy();
    });
  });

  it('should display token counts in compact format', async () => {
    const { getByText } = render(<AccountScreen />);

    await waitFor(() => {
      // 145000 -> "145.0K" and 500000 -> "500.0K"
      expect(getByText('145.0K / 500.0K')).toBeTruthy();
    });
  });

  it('should display resource counts for agents and conversations', async () => {
    const { getByText } = render(<AccountScreen />);

    await waitFor(() => {
      expect(getByText('Agents')).toBeTruthy();
      expect(getByText('2 / 3')).toBeTruthy();
      expect(getByText('Conversations')).toBeTruthy();
      expect(getByText('5 / 10')).toBeTruthy();
    });
  });
});
