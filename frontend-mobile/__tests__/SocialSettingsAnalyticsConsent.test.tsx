// ABOUTME: Unit tests for the analytics-consent control on the social/privacy settings screen
// ABOUTME: Pins the optimistic toggle and, critically, that it reverts when the write fails

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';
import type { User } from '@pierre/shared-types';

jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: jest.fn() }),
  // Run once, not on every render — the screen loads settings in here and sets
  // state, so a bare cb() re-enters forever.
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

const mockUpdateAnalyticsConsent = jest.fn();
jest.mock('../src/services/api', () => ({
  socialApi: {
    getSocialSettings: jest.fn().mockResolvedValue({
      settings: {
        discoverable: true,
        default_visibility: 'friends_only',
        notifications: {},
      },
    }),
    updateSocialSettings: jest.fn(),
  },
  userApi: {
    updateAnalyticsConsent: (enabled: boolean) => mockUpdateAnalyticsConsent(enabled),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { SocialSettingsScreen } from '../src/screens/social/SocialSettingsScreen';

const userWithConsent = (consent: boolean): Partial<User> => ({
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  analytics_consent: consent,
});

describe('SocialSettingsScreen — analytics consent', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(false) as User,
      updateUser: mockUpdateUser,
    });
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Updated', enabled: true });
  });

  it('seeds the switch from the stored consent flag', async () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(true) as User,
      updateUser: mockUpdateUser,
    });
    const { getByTestId } = render(<SocialSettingsScreen />);
    await waitFor(() => {
      expect(getByTestId('analytics-consent-switch').props.value).toBe(true);
    });
  });

  it('persists an opt-in and reflects it on the user record', async () => {
    const { getByTestId } = render(<SocialSettingsScreen />);
    await waitFor(() => expect(getByTestId('analytics-consent-switch')).toBeTruthy());

    fireEvent(getByTestId('analytics-consent-switch'), 'valueChange', true);

    await waitFor(() => {
      expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true);
    });
    await waitFor(() => {
      expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true });
    });
  });

  it('reverts the switch when the write fails', async () => {
    // The important one. An optimistic switch that stays flipped after a failed
    // write tells the user their data sharing is off while it is still on.
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockUpdateAnalyticsConsent.mockRejectedValueOnce(new Error('network down'));

    const { getByTestId } = render(<SocialSettingsScreen />);
    await waitFor(() => expect(getByTestId('analytics-consent-switch')).toBeTruthy());

    fireEvent(getByTestId('analytics-consent-switch'), 'valueChange', true);

    await waitFor(() => {
      expect(alertSpy).toHaveBeenCalledWith('Could not save preference', 'network down');
    });
    await waitFor(() => {
      expect(getByTestId('analytics-consent-switch').props.value).toBe(false);
    });
    expect(mockUpdateUser).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });
});
