// ABOUTME: Unit tests pinning that every Settings row actually navigates somewhere
// ABOUTME: Six rows rendered a chevron and did nothing; these fail if any regresses to that

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Linking } from 'react-native';
import type { User } from '@pierre/shared-types';

const mockPush = jest.fn();
const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: mockPush, back: mockBack }),
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

import { SettingsScreen } from '../src/screens/settings/SettingsScreen';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

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

  // Each of these rendered a chevron-right promising navigation while its
  // TouchableOpacity carried no onPress at all. Tapping did nothing, which is
  // indistinguishable from a broken screen.
  it.each([
    ['settings-edit-profile-button', '/(app)/(tabs)/(settings)/profile'],
    ['settings-personal-info-button', '/(app)/(tabs)/(settings)/profile'],
    ['settings-memory-button', '/(app)/memory'],
    ['settings-privacy-button', '/(app)/(tabs)/(social)/social-settings'],
  ])('%s navigates to %s', (testID, destination) => {
    const { getByTestId } = render(<SettingsScreen />);
    fireEvent.press(getByTestId(testID));
    expect(mockPush).toHaveBeenCalledWith(destination);
  });

  it('routes both profile entry points to the same screen', () => {
    // Web exposes a single Profile tab. Two rows reaching two different places
    // would be the drift this parity work exists to remove.
    const { getByTestId } = render(<SettingsScreen />);
    fireEvent.press(getByTestId('settings-edit-profile-button'));
    fireEvent.press(getByTestId('settings-personal-info-button'));

    const destinations = mockPush.mock.calls.map((call) => call[0]);
    expect(destinations).toHaveLength(2);
    expect(new Set(destinations).size).toBe(1);
  });

  it('opens the help centre externally rather than doing nothing', async () => {
    const openSpy = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const { getByTestId } = render(<SettingsScreen />);
    fireEvent.press(getByTestId('settings-help-center-button'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith('https://dravr.ai/help');
    });
    openSpy.mockRestore();
  });

  it('opens terms externally rather than doing nothing', async () => {
    const openSpy = jest.spyOn(Linking, 'openURL').mockResolvedValue(true);
    const { getByTestId } = render(<SettingsScreen />);
    fireEvent.press(getByTestId('settings-terms-privacy-button'));
    await waitFor(() => {
      expect(openSpy).toHaveBeenCalledWith('https://dravr.ai/privacy');
    });
    openSpy.mockRestore();
  });

  it('renders the version as a plain row, not a tappable control', () => {
    // Version has no destination and never did. Presenting it as a button
    // advertised a tap that could not be serviced.
    const { getByTestId, queryByTestId } = render(<SettingsScreen />);
    expect(getByTestId('settings-version-row')).toBeTruthy();
    expect(queryByTestId('settings-version-button')).toBeNull();
  });

  it('hides the billing row while billing is disabled', () => {
    // BILLING_ENABLED is false for the first release and the billing route
    // redirects away; showing an entry point would strand the user.
    const { queryByTestId } = render(<SettingsScreen />);
    expect(queryByTestId('settings-billing-button')).toBeNull();
  });
});
