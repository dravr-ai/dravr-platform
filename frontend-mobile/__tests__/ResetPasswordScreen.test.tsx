// ABOUTME: Unit tests for ResetPasswordScreen component
// ABOUTME: Verifies code entry, password validation, API calls, and navigation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

// Mock navigation and route
const mockNavigate = jest.fn();
const mockNavigation = {
  navigate: mockNavigate,
  goBack: jest.fn(),
};

const mockRoute = {
  params: { email: 'test@example.com' },
  key: 'ResetPassword',
  name: 'ResetPassword' as const,
};

// Mock API service
const mockResetPassword = jest.fn();

jest.mock('../src/services/api', () => ({
  authApi: {
    resetPassword: (...args: unknown[]) => mockResetPassword(...args),
  },
}));

// Mock @expo/vector-icons
jest.mock('@expo/vector-icons', () => {
  const View = require('react-native').View;
  return {
    Ionicons: (props: Record<string, unknown>) =>
      require('react').createElement(View, { testID: `icon-${props.name}` }),
  };
});

import { ResetPasswordScreen } from '../src/screens/auth/ResetPasswordScreen';

describe('ResetPasswordScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => {});
  });

  function renderComponent() {
    return render(
      <ResetPasswordScreen
        navigation={mockNavigation as never}
        route={mockRoute as never}
      />,
    );
  }

  describe('rendering', () => {
    it('should render the heading', () => {
      const { getByText } = renderComponent();
      expect(getByText('Enter Reset Code')).toBeTruthy();
    });

    it('should display the email address', () => {
      const { getByText } = renderComponent();
      expect(getByText(/test@example\.com/)).toBeTruthy();
    });

    it('should render code, password, and confirm inputs', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('reset-code-input')).toBeTruthy();
      expect(getByTestId('new-password-input')).toBeTruthy();
      expect(getByTestId('confirm-password-input')).toBeTruthy();
    });

    it('should render reset button and action links', () => {
      const { getByTestId, getByText } = renderComponent();
      expect(getByTestId('reset-password-button')).toBeTruthy();
      expect(getByText('Resend code')).toBeTruthy();
      expect(getByText('Back to sign in')).toBeTruthy();
    });
  });

  describe('form validation', () => {
    it('should reject empty code', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      expect(getByText('Reset code is required')).toBeTruthy();
      expect(mockResetPassword).not.toHaveBeenCalled();
    });

    it('should reject code shorter than 6 digits', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123');
      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      expect(getByText('Please enter a valid 6-digit code')).toBeTruthy();
      expect(mockResetPassword).not.toHaveBeenCalled();
    });

    it('should reject short password', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'short');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'short');
      fireEvent.press(getByTestId('reset-password-button'));

      expect(getByText('Password must be at least 8 characters')).toBeTruthy();
      expect(mockResetPassword).not.toHaveBeenCalled();
    });

    it('should reject mismatched passwords', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'ValidPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'DifferentPassword456');
      fireEvent.press(getByTestId('reset-password-button'));

      expect(getByText('Passwords do not match')).toBeTruthy();
      expect(mockResetPassword).not.toHaveBeenCalled();
    });
  });

  describe('form submission', () => {
    it('should call resetPassword API on valid submit', async () => {
      mockResetPassword.mockResolvedValueOnce({ message: 'Password reset' });
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      await waitFor(() => {
        expect(mockResetPassword).toHaveBeenCalledWith('123456', 'NewPassword123');
      });
    });

    it('should show success alert and navigate to Login', async () => {
      mockResetPassword.mockResolvedValueOnce({ message: 'Password reset' });
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Password Reset',
          'Your password has been reset successfully. Please sign in.',
          expect.arrayContaining([
            expect.objectContaining({ text: 'OK' }),
          ]),
        );
      });
    });

    it('should show expired code error on 404', async () => {
      mockResetPassword.mockRejectedValueOnce(
        new Error('Request failed with status 404: not found'),
      );
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith(
          'Reset Failed',
          'Invalid or expired code. Please request a new one.',
        );
      });
    });

    it('should show generic error for other failures', async () => {
      mockResetPassword.mockRejectedValueOnce(new Error('Server error'));
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('reset-code-input'), '123456');
      fireEvent.changeText(getByTestId('new-password-input'), 'NewPassword123');
      fireEvent.changeText(getByTestId('confirm-password-input'), 'NewPassword123');
      fireEvent.press(getByTestId('reset-password-button'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Reset Failed', 'Server error');
      });
    });
  });

  describe('navigation', () => {
    it('should navigate to ForgotPassword when resend code is pressed', () => {
      const { getByText } = renderComponent();

      fireEvent.press(getByText('Resend code'));

      expect(mockNavigate).toHaveBeenCalledWith('ForgotPassword');
    });

    it('should navigate to Login when back link is pressed', () => {
      const { getByText } = renderComponent();

      fireEvent.press(getByText('Back to sign in'));

      expect(mockNavigate).toHaveBeenCalledWith('Login');
    });
  });
});
