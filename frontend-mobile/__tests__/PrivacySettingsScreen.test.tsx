// ABOUTME: Unit tests for the analytics-consent control on the Settings privacy screen
// ABOUTME: Pins that it reads the stored flag, writes through userApi, and reverts when the write fails

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { User } from '@pierre/shared-types';

const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ push: jest.fn(), back: mockBack }),
}));

const mockUpdateAnalyticsConsent = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    updateAnalyticsConsent: (enabled: boolean) => mockUpdateAnalyticsConsent(enabled),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { PrivacySettingsScreen } from '../src/screens/settings/PrivacySettingsScreen';

const userWithConsent = (consent: boolean): Partial<User> => ({
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  analytics_consent: consent,
});

function renderScreen() {
  const queryClient = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <PrivacySettingsScreen />
    </QueryClientProvider>,
  );
}

describe('PrivacySettingsScreen — analytics consent', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(false) as User,
      updateUser: mockUpdateUser,
    });
    mockUpdateAnalyticsConsent.mockResolvedValue({ message: 'Updated', enabled: true });
    mockUpdateUser.mockResolvedValue(undefined);
  });

  it('renders under Settings, not the social group', () => {
    const { getByTestId, getByText } = renderScreen();
    expect(getByTestId('privacy-settings-screen')).toBeTruthy();
    expect(getByText('Privacy & Data')).toBeTruthy();
    fireEvent.press(getByTestId('back-button'));
    expect(mockBack).toHaveBeenCalledTimes(1);
  });

  it('seeds the switch from the stored consent flag', () => {
    mockUseAuth.mockReturnValue({
      isAuthenticated: true,
      user: userWithConsent(true) as User,
      updateUser: mockUpdateUser,
    });
    const { getByTestId } = renderScreen();
    expect(getByTestId('analytics-consent-switch').props.value).toBe(true);
  });

  it('persists an opt-in and reflects it on the user record', async () => {
    const { getByTestId } = renderScreen();
    expect(getByTestId('analytics-consent-switch').props.value).toBe(false);

    fireEvent(getByTestId('analytics-consent-switch'), 'valueChange', true);

    await waitFor(() => {
      expect(mockUpdateAnalyticsConsent).toHaveBeenCalledWith(true);
    });
    await waitFor(() => {
      expect(mockUpdateUser).toHaveBeenCalledWith({ analytics_consent: true });
    });
    expect(getByTestId('analytics-consent-switch').props.value).toBe(true);
  });

  it('reverts the switch when the write fails', async () => {
    // The important one. An optimistic switch that stays flipped after a failed
    // write tells the user their data sharing is off while it is still on.
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockUpdateAnalyticsConsent.mockRejectedValueOnce(new Error('network down'));

    const { getByTestId } = renderScreen();
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
