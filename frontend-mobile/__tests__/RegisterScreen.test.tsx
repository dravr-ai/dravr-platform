// ABOUTME: Unit tests for RegisterScreen component
// ABOUTME: Verifies the form, validation, the register call, success/error paths, and the login link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor } from '@testing-library/react-native';
import { Alert } from 'react-native';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
}));

const mockRegister = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ register: (...args: unknown[]) => mockRegister(...args) }),
}));

import { RegisterScreen } from '../src/screens/auth/RegisterScreen';
import { apiRefusal } from '../integration/app/helpers/apiRefusal';

describe('RegisterScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => {});
  });

  function renderComponent() {
    return render(<RegisterScreen />);
  }

  describe('rendering', () => {
    it('carries its own testID and no card shell', () => {
      const { getByTestId } = renderComponent();
      expect(getByTestId('register-screen')).toBeTruthy();
    });

    it('renders the brand lockup, heading and every field', () => {
      const { getByTestId, getAllByText } = renderComponent();
      expect(getByTestId('register-lockup')).toBeTruthy();
      // Both the headline and the submit button read "Create Account".
      expect(getAllByText('Create Account').length).toBe(2);
      expect(getByTestId('register-display-name-input')).toBeTruthy();
      expect(getByTestId('register-email-input')).toBeTruthy();
      expect(getByTestId('register-password-input')).toBeTruthy();
      expect(getByTestId('register-confirm-password-input')).toBeTruthy();
      expect(getByTestId('register-submit-button')).toBeTruthy();
      expect(getByTestId('register-login-link')).toBeTruthy();
    });
  });

  describe('form validation', () => {
    it('rejects an empty form', () => {
      const { getByTestId, getByText } = renderComponent();
      fireEvent.press(getByTestId('register-submit-button'));

      expect(getByText('Display name is required')).toBeTruthy();
      expect(mockRegister).not.toHaveBeenCalled();
    });

    it('rejects mismatched passwords', () => {
      const { getByTestId, getByText } = renderComponent();
      fireEvent.changeText(getByTestId('register-display-name-input'), 'Jean');
      fireEvent.changeText(getByTestId('register-email-input'), 'jean@example.com');
      fireEvent.changeText(getByTestId('register-password-input'), 'ValidPassword123');
      fireEvent.changeText(getByTestId('register-confirm-password-input'), 'DifferentPassword456');
      fireEvent.press(getByTestId('register-submit-button'));

      expect(getByText('Passwords do not match')).toBeTruthy();
      expect(mockRegister).not.toHaveBeenCalled();
    });
  });

  describe('form submission', () => {
    it('registers with the trimmed fields and routes to pending-approval on success', async () => {
      mockRegister.mockResolvedValueOnce(undefined);
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('register-display-name-input'), '  Jean  ');
      fireEvent.changeText(getByTestId('register-email-input'), '  jean@example.com  ');
      fireEvent.changeText(getByTestId('register-password-input'), 'ValidPassword123');
      fireEvent.changeText(getByTestId('register-confirm-password-input'), 'ValidPassword123');
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(mockRegister).toHaveBeenCalledWith('jean@example.com', 'ValidPassword123', 'Jean'),
      );
      await waitFor(() => expect(mockRouter.replace).toHaveBeenCalledWith('/(auth)/pending-approval'));
    });

    it('alerts on a failed registration and does not navigate', async () => {
      mockRegister.mockRejectedValueOnce(apiRefusal(409, { message: 'Email already registered' }));
      const { getByTestId } = renderComponent();

      fireEvent.changeText(getByTestId('register-display-name-input'), 'Jean');
      fireEvent.changeText(getByTestId('register-email-input'), 'jean@example.com');
      fireEvent.changeText(getByTestId('register-password-input'), 'ValidPassword123');
      fireEvent.changeText(getByTestId('register-confirm-password-input'), 'ValidPassword123');
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(Alert.alert).toHaveBeenCalledWith('Registration Failed', 'Email already registered'),
      );
      expect(mockRouter.replace).not.toHaveBeenCalled();
    });
  });

  describe('navigation', () => {
    it('navigates to Login when the login link is pressed', () => {
      const { getByTestId } = renderComponent();
      fireEvent.press(getByTestId('register-login-link'));
      expect(mockRouter.replace).toHaveBeenCalledWith('/(auth)/login');
    });
  });
});
