// ABOUTME: Unit tests for CoachingStyleScreen component
// ABOUTME: Tests render, persona selection, optimistic update, and rollback on error

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import type { CoachingPersona, User } from '@pierre/shared-types';

const mockBack = jest.fn();
jest.mock('expo-router', () => ({
  useRouter: () => ({ back: mockBack, push: jest.fn() }),
}));

const mockSetCoachingPersona = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    setCoachingPersona: (persona: CoachingPersona) => mockSetCoachingPersona(persona),
  },
}));

const mockUpdateUser = jest.fn();
const mockUseAuth = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockUseAuth(),
}));

import { CoachingStyleScreen } from '../src/screens/settings/CoachingStyleScreen';

const baseUser: Partial<User> = {
  id: 'user-1',
  email: 'mobile@example.com',
  is_admin: false,
  role: 'user',
  user_status: 'active',
  coaching_persona: 'casual',
};

describe('CoachingStyleScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseAuth.mockReturnValue({
      user: baseUser as User,
      updateUser: mockUpdateUser,
    });
  });

  it('renders all four persona cards', () => {
    const { getByTestId } = render(<CoachingStyleScreen />);
    expect(getByTestId('persona-card-casual')).toBeTruthy();
    expect(getByTestId('persona-card-enthusiast')).toBeTruthy();
    expect(getByTestId('persona-card-power_athlete')).toBeTruthy();
    expect(getByTestId('persona-card-coach')).toBeTruthy();
  });

  it('marks the user current persona as active on mount', () => {
    mockUseAuth.mockReturnValue({
      user: { ...baseUser, coaching_persona: 'power_athlete' } as User,
      updateUser: mockUpdateUser,
    });
    const { getByTestId } = render(<CoachingStyleScreen />);
    expect(getByTestId('persona-card-power_athlete').props.accessibilityState.selected).toBe(true);
    expect(getByTestId('persona-card-casual').props.accessibilityState.selected).toBe(false);
  });

  it('calls userApi.setCoachingPersona and updateUser on selection', async () => {
    mockSetCoachingPersona.mockResolvedValueOnce({ message: 'ok', persona: 'enthusiast' });
    const { getByTestId } = render(<CoachingStyleScreen />);

    fireEvent.press(getByTestId('persona-card-enthusiast'));

    await waitFor(() => {
      expect(mockSetCoachingPersona).toHaveBeenCalledWith('enthusiast');
    });
    expect(mockUpdateUser).toHaveBeenCalledWith({ coaching_persona: 'enthusiast' });
  });

  it('rolls back the selected card on mutation error', async () => {
    mockSetCoachingPersona.mockRejectedValueOnce(new Error('500 internal'));
    const { getByTestId } = render(<CoachingStyleScreen />);

    fireEvent.press(getByTestId('persona-card-coach'));

    await waitFor(() => {
      expect(getByTestId('persona-status')).toBeTruthy();
    });
    // Selection rolls back to the original persona.
    expect(getByTestId('persona-card-casual').props.accessibilityState.selected).toBe(true);
    expect(getByTestId('persona-card-coach').props.accessibilityState.selected).toBe(false);
    expect(mockUpdateUser).not.toHaveBeenCalled();
  });

  it('does not refire the mutation when tapping the already-selected card', () => {
    const { getByTestId } = render(<CoachingStyleScreen />);
    fireEvent.press(getByTestId('persona-card-casual'));
    expect(mockSetCoachingPersona).not.toHaveBeenCalled();
  });

  it('navigates back when the back button is pressed', () => {
    const { getByTestId } = render(<CoachingStyleScreen />);
    fireEvent.press(getByTestId('back-button'));
    expect(mockBack).toHaveBeenCalled();
  });
});
