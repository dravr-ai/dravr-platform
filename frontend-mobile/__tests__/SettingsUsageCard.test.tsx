// ABOUTME: Unit tests for the Usage section in the SettingsScreen
// ABOUTME: Verifies quota meter labels, compact formatting, and resource counts

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';

// Mock navigation
const mockNavigation = {
  navigate: jest.fn(),
  goBack: jest.fn(),
};

// Mock useFocusEffect
jest.mock('@react-navigation/native', () => {
  const actualReact = jest.requireActual('react');
  return {
    useFocusEffect: (callback: () => (() => void) | void) => {
      actualReact.useEffect(() => {
        return callback();
      }, [callback]);
    },
    useNavigation: () => mockNavigation,
  };
});

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

// Mock AuthContext
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({
    user: {
      id: 'user-1',
      email: 'test@pierre.dev',
      display_name: 'Test User',
      role: 'user',
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
import { SettingsScreen } from '../src/screens/settings/SettingsScreen';
import type { NativeStackNavigationProp } from '@react-navigation/native-stack';
import type { SettingsStackParamList } from '../src/navigation/MainTabs';

type Nav = NativeStackNavigationProp<SettingsStackParamList>;

describe('SettingsScreen - Usage Card', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('should render usage section with quota meter labels', async () => {
    const { getByText } = render(
      <SettingsScreen navigation={mockNavigation as unknown as Nav} />
    );

    await waitFor(() => {
      expect(getByText('Usage')).toBeTruthy();
      expect(getByText('Daily Messages')).toBeTruthy();
      expect(getByText('Daily Tokens')).toBeTruthy();
      expect(getByText('Weekly Messages')).toBeTruthy();
    });
  });

  it('should display token counts in compact format', async () => {
    const { getByText } = render(
      <SettingsScreen navigation={mockNavigation as unknown as Nav} />
    );

    await waitFor(() => {
      // 145000 -> "145.0K" and 500000 -> "500.0K"
      expect(getByText('145.0K / 500.0K')).toBeTruthy();
    });
  });

  it('should display resource counts for coaches and conversations', async () => {
    const { getByText } = render(
      <SettingsScreen navigation={mockNavigation as unknown as Nav} />
    );

    await waitFor(() => {
      expect(getByText('Coaches')).toBeTruthy();
      expect(getByText('2 / 3')).toBeTruthy();
      expect(getByText('Conversations')).toBeTruthy();
      expect(getByText('5 / 10')).toBeTruthy();
    });
  });
});
