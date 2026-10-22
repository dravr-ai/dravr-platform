// ABOUTME: Unit tests for RegisterScreen component
// ABOUTME: Verifies the form, validation, the register call, success/error paths, the sign-up guards, and the login link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import React from 'react';
import { render, fireEvent, waitFor, act, type RenderResult } from '@testing-library/react-native';
import { Alert } from 'react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
}));

const mockRegister = jest.fn();
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ register: (...args: unknown[]) => mockRegister(...args) }),
}));

const mockGetSignupConfig = jest.fn();
jest.mock('../src/services/api', () => ({
  authApi: { getSignupConfig: (...args: unknown[]) => mockGetSignupConfig(...args) },
}));

// The WebView is native; the stub keeps its props so a test can post the
// page's messages and see what the screen injects.
const mockWebViewProps: { current: Record<string, unknown> | null } = { current: null };
const mockInjectJavaScript = jest.fn();
jest.mock('react-native-webview', () => {
  const ReactMock = require('react');
  const { View } = require('react-native');
  const WebView = ReactMock.forwardRef((props: Record<string, unknown>, ref: unknown) => {
    mockWebViewProps.current = props;
    ReactMock.useImperativeHandle(ref, () => ({ injectJavaScript: mockInjectJavaScript }));
    return ReactMock.createElement(View, { testID: props.testID });
  });
  return { WebView };
});

import { RegisterScreen } from '../src/screens/auth/RegisterScreen';
import { apiRefusal } from '../integration/app/helpers/apiRefusal';

describe('RegisterScreen', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    jest.spyOn(Alert, 'alert').mockImplementation(() => {});
    mockWebViewProps.current = null;
    mockGetSignupConfig.mockResolvedValue({ open: true, turnstile_site_key: null });
  });

  function renderComponent() {
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    return render(
      <QueryClientProvider client={queryClient}>
        <RegisterScreen />
      </QueryClientProvider>,
    );
  }

  /** Render, then wait for the sign-up config: the form holds submit until it is in. */
  async function renderReady() {
    const view = renderComponent();
    await waitFor(() => expect(view.getByTestId('register-submit-button')).toBeEnabled());
    return view;
  }

  function fillForm(getByTestId: RenderResult['getByTestId']) {
    fireEvent.changeText(getByTestId('register-display-name-input'), 'Jean');
    fireEvent.changeText(getByTestId('register-email-input'), 'jean@example.com');
    fireEvent.changeText(getByTestId('register-password-input'), 'ValidPassword123');
    fireEvent.changeText(getByTestId('register-confirm-password-input'), 'ValidPassword123');
  }

  /** Post a message as the Turnstile page would. */
  function postFromPage(message: Record<string, unknown>) {
    const onMessage = mockWebViewProps.current?.onMessage as (event: { nativeEvent: { data: string } }) => void;
    act(() => onMessage({ nativeEvent: { data: JSON.stringify(message) } }));
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
    it('rejects an empty form', async () => {
      const { getByTestId, getByText } = await renderReady();
      fireEvent.press(getByTestId('register-submit-button'));

      expect(getByText('Display name is required')).toBeTruthy();
      expect(mockRegister).not.toHaveBeenCalled();
    });

    it('rejects mismatched passwords', async () => {
      const { getByTestId, getByText } = await renderReady();
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
    it('registers with the trimmed fields and routes to pending-approval with the server sentence', async () => {
      mockRegister.mockResolvedValueOnce({
        user_id: 'u1',
        email: 'jean@example.com',
        user_status: 'pending',
        message: 'Confirm your email to activate your account.',
      });
      const { getByTestId } = await renderReady();

      fireEvent.changeText(getByTestId('register-display-name-input'), '  Jean  ');
      fireEvent.changeText(getByTestId('register-email-input'), '  jean@example.com  ');
      fireEvent.changeText(getByTestId('register-password-input'), 'ValidPassword123');
      fireEvent.changeText(getByTestId('register-confirm-password-input'), 'ValidPassword123');
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(mockRegister).toHaveBeenCalledWith('jean@example.com', 'ValidPassword123', 'Jean', undefined, undefined),
      );
      await waitFor(() =>
        expect(mockRouter.replace).toHaveBeenCalledWith({
          pathname: '/(auth)/pending-approval',
          params: { message: 'Confirm your email to activate your account.' },
        }),
      );
    });

    it('alerts on a failed registration and does not navigate', async () => {
      mockRegister.mockRejectedValueOnce(apiRefusal(409, { message: 'Email already registered' }));
      const { getByTestId } = await renderReady();

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

  // A code handed out by an admin or a coach approves the account at once
  // (carnet#1128); the field stays folded until the athlete asks for it.
  describe('signup code', () => {
    it('keeps the code field folded until asked for', async () => {
      const { getByTestId, queryByTestId } = await renderReady();
      expect(queryByTestId('register-signup-code-input')).toBeNull();

      fireEvent.press(getByTestId('register-signup-code-toggle'));

      const input = getByTestId('register-signup-code-input');
      expect(input.props.autoCapitalize).toBe('characters');
      expect(input.props.autoCorrect).toBe(false);
    });

    it('sends the trimmed code as the signup code', async () => {
      mockRegister.mockResolvedValueOnce({ user_id: 'u1', user_status: 'active', message: 'Welcome.' });
      const { getByTestId } = await renderReady();

      fillForm(getByTestId);
      fireEvent.press(getByTestId('register-signup-code-toggle'));
      fireEvent.changeText(getByTestId('register-signup-code-input'), '  BETA-ABCD-EFGH  ');
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(mockRegister).toHaveBeenCalledWith(
          'jean@example.com',
          'ValidPassword123',
          'Jean',
          undefined,
          'BETA-ABCD-EFGH',
        ),
      );
    });

    it('omits a code left blank', async () => {
      mockRegister.mockResolvedValueOnce({ user_id: 'u1', user_status: 'pending', message: 'Pending.' });
      const { getByTestId } = await renderReady();

      fillForm(getByTestId);
      fireEvent.press(getByTestId('register-signup-code-toggle'));
      fireEvent.changeText(getByTestId('register-signup-code-input'), '   ');
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(mockRegister).toHaveBeenCalledWith('jean@example.com', 'ValidPassword123', 'Jean', undefined, undefined),
      );
    });

    it('shows a refused code next to its field, not in an alert', async () => {
      mockRegister.mockRejectedValueOnce(
        apiRefusal(400, {
          message: "This code isn't valid. Check it and try again.",
          details: { reason: 'invalid_signup_code' },
        }),
      );
      const { getByTestId, findByText } = await renderReady();

      fillForm(getByTestId);
      fireEvent.press(getByTestId('register-signup-code-toggle'));
      fireEvent.changeText(getByTestId('register-signup-code-input'), 'NOPE');
      fireEvent.press(getByTestId('register-submit-button'));

      expect(await findByText("This code isn't valid. Check it and try again.")).toBeTruthy();
      expect(Alert.alert).not.toHaveBeenCalled();
      expect(mockRouter.replace).not.toHaveBeenCalled();
    });
  });

  describe('sign-up guards', () => {
    it('shows the paused notice instead of the form while sign-up is closed', async () => {
      mockGetSignupConfig.mockResolvedValue({ open: false, turnstile_site_key: null });
      const { findByTestId, queryByTestId, getByTestId } = renderComponent();

      expect(await findByTestId('register-signup-paused')).toBeTruthy();
      expect(queryByTestId('register-email-input')).toBeNull();
      expect(queryByTestId('register-submit-button')).toBeNull();
      // The way back to sign-in stays.
      expect(getByTestId('register-login-link')).toBeTruthy();
    });

    it('keeps the plain form when the sign-up config cannot be read', async () => {
      mockGetSignupConfig.mockRejectedValue(new Error('404'));
      const { queryByTestId, getByTestId } = renderComponent();

      // The read is retried once (a 1 s backoff) before the form gives up on it.
      await waitFor(() => expect(mockGetSignupConfig).toHaveBeenCalledTimes(2), { timeout: 3000 });
      await waitFor(() => expect(getByTestId('register-submit-button')).toBeEnabled());

      expect(queryByTestId('register-signup-paused')).toBeNull();
      expect(queryByTestId('turnstile-webview')).toBeNull();
    });

    it('holds submit until Turnstile passes, then sends its token', async () => {
      mockGetSignupConfig.mockResolvedValue({ open: true, turnstile_site_key: '1x00000000000000000000AA' });
      mockRegister.mockResolvedValueOnce({ user_id: 'u1', email: 'jean@example.com', user_status: 'pending', message: 'ok' });
      const { findByTestId, getByTestId } = renderComponent();

      expect(await findByTestId('turnstile-webview')).toBeTruthy();
      const source = mockWebViewProps.current?.source as { html: string; baseUrl: string };
      expect(source.html).toContain('"1x00000000000000000000AA"');
      expect(source.html).toContain('render=explicit');
      expect(source.baseUrl).toMatch(/^https?:\/\/[^/]+\/$/);

      fillForm(getByTestId);
      expect(getByTestId('register-submit-button')).toBeDisabled();
      fireEvent.press(getByTestId('register-submit-button'));
      expect(mockRegister).not.toHaveBeenCalled();

      postFromPage({ type: 'token', token: 'tok-1' });
      await waitFor(() => expect(getByTestId('register-submit-button')).toBeEnabled());
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(mockRegister).toHaveBeenCalledWith('jean@example.com', 'ValidPassword123', 'Jean', 'tok-1', undefined),
      );
    });

    it('resets the check after a failed human check and waits for a new token', async () => {
      mockGetSignupConfig.mockResolvedValue({ open: true, turnstile_site_key: '1x00000000000000000000AA' });
      mockRegister.mockRejectedValueOnce(
        apiRefusal(403, { code: 'PermissionDenied', message: 'The human check failed. Please try again.' }),
      );
      const { findByTestId, getByTestId } = renderComponent();

      await findByTestId('turnstile-webview');
      fillForm(getByTestId);
      postFromPage({ type: 'token', token: 'tok-spent' });
      await waitFor(() => expect(getByTestId('register-submit-button')).toBeEnabled());
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() =>
        expect(Alert.alert).toHaveBeenCalledWith('Registration Failed', 'The human check failed. Please try again.'),
      );
      expect(mockInjectJavaScript).toHaveBeenCalledWith(expect.stringContaining('turnstile.reset()'));
      expect(getByTestId('register-submit-button')).toBeDisabled();
    });

    it.each([
      [429, 'QuotaExceeded', 'Sign-up is paused right now. Please try again later.'],
      [429, 'RateLimitExceeded', 'Too many sign-ups from this network. Please try again later.'],
      [403, 'PermissionDenied', 'We could not confirm you are not a bot. Complete the check again, then retry.'],
    ])('shows the server sentence for a %i refusal', async (status, code, message) => {
      mockRegister.mockRejectedValueOnce(apiRefusal(status, { code, message }));
      const { getByTestId } = await renderReady();

      fillForm(getByTestId);
      fireEvent.press(getByTestId('register-submit-button'));

      await waitFor(() => expect(Alert.alert).toHaveBeenCalledWith('Registration Failed', message));
      expect(mockRouter.replace).not.toHaveBeenCalled();
    });
  });

  describe('navigation', () => {
    it('navigates to Login when the login link is pressed', async () => {
      const { getByTestId } = await renderReady();
      fireEvent.press(getByTestId('register-login-link'));
      expect(mockRouter.replace).toHaveBeenCalledWith('/(auth)/login');
    });
  });
});
