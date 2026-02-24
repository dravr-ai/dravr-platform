// ABOUTME: Unit tests for ForgotPasswordScreen component
// ABOUTME: Verifies email form, validation, API calls, navigation, and error handling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

// Mock navigation
const mockNavigate = jest.fn();
const mockNavigation = {
  navigate: mockNavigate,
  goBack: jest.fn(),
};

// Mock API service
const mockForgotPassword = jest.fn();

jest.mock('../src/services/api', () => ({
  authApi: {
    forgotPassword: (...args: unknown[]) => mockForgotPassword(...args),
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

import { ForgotPasswordScreen } from '../src/screens/auth/ForgotPasswordScreen';

describe('ForgotPasswordScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => {});
  });

  function renderComponent() {
    return render(
      <ForgotPasswordScreen navigation={mockNavigation as never} />,
    );
  }

  describe('rendering', () => {
    it('should render the heading and description', () => {
      const { getByText } = renderComponent();
      expect(getByText('Reset Your Password')).toBeTruthy();
      expect(getByText(/6-digit code/)).toBeTruthy();
    });

    it('should render the email input', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('forgot-email-input')).toBeTruthy();
    });

    it('should render the send code button', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('send-code-button')).toBeTruthy();
    });

    it('should render back to sign in link', () => {
      const { getByText } = renderComponent();
      expect(getByText('Back to sign in')).toBeTruthy();
    });
  });

  describe('form validation', () => {
    it('should show error when email is empty', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.press(getByTestId('send-code-button'));

      expect(getByText('Email is required')).toBeTruthy();
      expect(mockForgotPassword).not.toHaveBeenCalled();
    });

    it('should show error for invalid email format', () => {
      const { getByTestId, getByText } = renderComponent();

      fireEvent.changeText(getByTestId('forgot-email-input'), 'not-an-email');
      fireEvent.press(getByTestId('send-code-button'));

      expect(getByText('Please enter a valid email')).toBeTruthy();
      expect(mockForgotPassword).not.toHaveBeenCalled();
    });
  });

  describe('form submission', () => {
    it('should call forgotPassword API on valid submit', async () => {
      mockForgotPassword.mockResolvedValueOnce({ message: 'Code sent' });
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('forgot-email-input'), 'test@example.com');
      fireEvent.press(getByTestId('send-code-button'));

      await waitFor(() => {
        expect(mockForgotPassword).toHaveBeenCalledWith('test@example.com');
      });
    });

    it('should navigate to ResetPassword on success', async () => {
      mockForgotPassword.mockResolvedValueOnce({ message: 'Code sent' });
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('forgot-email-input'), 'test@example.com');
      fireEvent.press(getByTestId('send-code-button'));

      await waitFor(() => {
        expect(mockNavigate).toHaveBeenCalledWith('ResetPassword', {
          email: 'test@example.com',
        });
      });
    });

    it('should show alert on API error', async () => {
      mockForgotPassword.mockRejectedValueOnce(new Error('Network error'));
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('forgot-email-input'), 'test@example.com');
      fireEvent.press(getByTestId('send-code-button'));

      await waitFor(() => {
        expect(Alert.alert).toHaveBeenCalledWith('Error', 'Network error');
      });
    });
  });

  describe('navigation', () => {
    it('should navigate to Login when back link is pressed', () => {
      const { getByText } = renderComponent();

      fireEvent.press(getByText('Back to sign in'));

      expect(mockNavigate).toHaveBeenCalledWith('Login');
    });
  });
});
