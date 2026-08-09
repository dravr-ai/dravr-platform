// ABOUTME: Unit tests for ProfileScreen — the destination both dead profile buttons now reach
// ABOUTME: Covers the dirty-state save gate, the update call, and rollback messaging on failure

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';
import type { User } from '@pierre/shared-types';

const mockBack = jest.fn();
const mockPush = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ back: mockBack, push: mockPush }),
}));

const mockUpdateProfile = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    updateProfile: (data: { display_name: string }) => mockUpdateProfile(data),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { ProfileScreen } from '../src/screens/settings/ProfileScreen';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobiletest@pierre.dev',
  display_name: 'Mobile Test User',
  is_admin: false,
  role: 'user',
  user_status: 'active',
};

describe('ProfileScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({ user: baseUser as User, updateUser: mockUpdateUser });
    mockUpdateProfile.mockResolvedValue({
      message: 'Profile updated',
      user: { ...baseUser, display_name: 'Renamed Athlete' },
    });
  });

  it('shows the current display name and the account email', () => {
    const { getByTestId, getAllByText } = render(<ProfileScreen />);
    expect(getByTestId('profile-display-name-input').props.value).toBe('Mobile Test User');
    // Email appears under the avatar and again in the read-only field.
    expect(getAllByText('mobiletest@pierre.dev').length).toBeGreaterThan(0);
  });

  it('does not call the API when the name is unchanged', () => {
    const { getByTestId } = render(<ProfileScreen />);
    fireEvent.press(getByTestId('profile-save-button'));
    // The web Save button is disabled while the value matches; mobile must not
    // fire a pointless write either.
    expect(mockUpdateProfile).not.toHaveBeenCalled();
  });

  it('does not treat a whitespace-only name as a change', () => {
    const { getByTestId } = render(<ProfileScreen />);
    fireEvent.changeText(getByTestId('profile-display-name-input'), '   ');
    fireEvent.press(getByTestId('profile-save-button'));
    expect(mockUpdateProfile).not.toHaveBeenCalled();
  });

  it('saves a trimmed display name and reflects it locally', async () => {
    const { getByTestId } = render(<ProfileScreen />);
    fireEvent.changeText(getByTestId('profile-display-name-input'), '  Renamed Athlete  ');
    fireEvent.press(getByTestId('profile-save-button'));

    await waitFor(() => {
      expect(mockUpdateProfile).toHaveBeenCalledWith({ display_name: 'Renamed Athlete' });
    });
    // Local reflection matters: the Settings header and avatar initial read from
    // the auth context, so a save that skipped this would look like a no-op.
    await waitFor(() => {
      expect(mockUpdateUser).toHaveBeenCalledWith({ display_name: 'Renamed Athlete' });
    });
  });

  it('surfaces a failed save instead of silently returning', async () => {
    const alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => undefined);
    mockUpdateProfile.mockRejectedValueOnce(new Error('Display name already taken'));

    const { getByTestId } = render(<ProfileScreen />);
    fireEvent.changeText(getByTestId('profile-display-name-input'), 'Renamed Athlete');
    fireEvent.press(getByTestId('profile-save-button'));

    await waitFor(() => {
      expect(alertSpy).toHaveBeenCalledWith('Could not save profile', 'Display name already taken');
    });
    // A failed save must not update local state or navigate away, or the user
    // is shown a name the server rejected.
    expect(mockUpdateUser).not.toHaveBeenCalled();
    expect(mockBack).not.toHaveBeenCalled();
    alertSpy.mockRestore();
  });
});
