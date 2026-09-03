// ABOUTME: Pins that every settings row leads to the pane the shared declaration names, and clears the chrome
// ABOUTME: The screen was one 1,200pt scroll clipped by the notch at the top and the tab bar at the bottom

import React from 'react';
import { render, fireEvent } from '@testing-library/react-native';
import type { User } from '@pierre/shared-types';
import { settingsPanesFor } from '@pierre/shared-constants';

const mockPush = jest.fn();
const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: mockBack }),
  useFocusEffect: () => undefined,
}));

// jest.setup.js mocks the safe-area context with a real phone's geometry —
// 44pt of status bar, 34pt of home indicator. Zeroes would let both layout
// assertions pass on a screen that clips on every real device.
const TOP_INSET = 44;
const BOTTOM_INSET = 34;

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

const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

// The API Tokens pane is gated on the shared `api_tokens` flag. These specs
// cover the rest of the list, so the flag hook answers with its off default
// rather than dragging a QueryClientProvider into every render.
jest.mock('../src/hooks/useFeatureFlags', () => ({
  useFeatureFlags: () => ({ flags: { api_tokens: false, billing_header: false }, known: [], isLoading: false, isError: false }),
  FEATURE_KEYS: { apiTokens: 'api_tokens', billingHeader: 'billing_header' },
}));

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';
import { tabBarBottomOffset } from '../src/components/ui/ExpandableTabBar';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

/** The panes a signed-in athlete gets with both feature flags off. */
const athletePanes = settingsPanesFor('mobile').filter((pane) => pane.flag === null);

describe('SettingsScreen navigation', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      user: baseUser as User,
      logout: jest.fn(),
      isAuthenticated: true,
      updateUser: jest.fn(),
    });
  });

  it.each(athletePanes.map((pane) => [pane.id, pane.mobile as string]))(
    'the %s row pushes %s',
    (id, destination) => {
      const { getByTestId } = render(<SettingsScreen />);
      fireEvent.press(getByTestId(`settings-pane-${id}`));
      expect(mockPush).toHaveBeenCalledWith(destination);
    },
  );

  it('lists exactly the panes the shared declaration serves this athlete', () => {
    // The list is derived, so a pane added to the declaration for web alone
    // shows up here as a missing row rather than as a screen nobody compared.
    const { queryByTestId } = render(<SettingsScreen />);
    for (const pane of athletePanes) {
      expect(queryByTestId(`settings-pane-${pane.id}`)).toBeTruthy();
    }
    // Flag-gated panes stay out while their flag is off: API tokens behind
    // `api_tokens`, billing behind the build-time toggle.
    expect(queryByTestId('settings-pane-tokens')).toBeNull();
    expect(queryByTestId('settings-pane-billing')).toBeNull();
  });

  it('no longer offers a per-athlete AI provider row', () => {
    // Nobody brings their own model. The row asked for provider API keys and
    // changed nothing about the coaching that followed.
    const { queryByTestId } = render(<SettingsScreen />);
    expect(queryByTestId('settings-pane-ai-provider')).toBeNull();
    expect(queryByTestId('settings-ai-provider-button')).toBeNull();
  });

  it('keeps the top of the screen clear of the status bar', () => {
    // The title and first card scrolled up behind the notch: the safe-area
    // inset was padding inside the scroll, so it moved with the content.
    const { getByTestId } = render(<SettingsScreen />);
    const header = getByTestId('settings-safe-header');
    expect(header.props.style.paddingTop).toBe(TOP_INSET);
  });

  it('clears the floating tab bar at the bottom of the scroll', () => {
    // The tab bar floats over the scroll with no scrim, so the last row sat
    // half-hidden behind it.
    const { getByTestId } = render(<SettingsScreen />);
    const scroll = getByTestId('settings-scroll');
    expect(scroll.props.contentContainerStyle.paddingBottom).toBe(
      tabBarBottomOffset(BOTTOM_INSET),
    );
    expect(tabBarBottomOffset(BOTTOM_INSET)).toBeGreaterThan(BOTTOM_INSET);
  });

  it('routes the header button to the profile pane the list also serves', () => {
    const { getByTestId } = render(<SettingsScreen />);
    fireEvent.press(getByTestId('settings-edit-profile-button'));
    fireEvent.press(getByTestId('settings-pane-profile'));

    const destinations = mockPush.mock.calls.map((call) => call[0]);
    expect(destinations).toHaveLength(2);
    expect(new Set(destinations).size).toBe(1);
  });
});
