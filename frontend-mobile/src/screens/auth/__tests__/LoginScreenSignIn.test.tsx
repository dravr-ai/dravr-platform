// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins the login screen's sign-in button to the hosted sign-in, and how each outcome reaches the athlete
// ABOUTME: No password field on the phone (carnet#787): a closed browser says nothing, a refusal is named in the catalogue's words

import React from 'react';
import { Alert } from 'react-native';
import { fireEvent, render, waitFor } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';

jest.mock('nativewind', () => ({
  useColorScheme: () => ({ colorScheme: 'light', setColorScheme: jest.fn() }),
}));

const mockRouter = { push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true };
jest.mock('expo-router', () => ({
  ...jest.requireActual('expo-router'),
  useRouter: () => mockRouter,
  useLocalSearchParams: () => ({}),
  useFocusEffect: (cb: () => void) => { require('react').useEffect(cb, []); },
}));

jest.mock('@expo/vector-icons', () => {
  const { View } = require('react-native');
  return { AntDesign: (props: Record<string, unknown>) => require('react').createElement(View, { testID: `icon-${props.name}` }) };
});

const mockSignIn = jest.fn();
jest.mock('../../../contexts/AuthContext', () => ({
  useAuth: () => ({
    signIn: () => mockSignIn(),
    loginWithFirebase: jest.fn(),
  }),
}));

import { LoginScreen } from '../LoginScreen';
import { SignInRefusedError } from '../../../utils/hostedSignIn';

describe('LoginScreen sign-in', () => {
  let alertSpy: jest.SpyInstance;

  beforeEach(() => {
    jest.clearAllMocks();
    alertSpy = jest.spyOn(Alert, 'alert').mockImplementation(() => {});
  });

  afterEach(() => {
    alertSpy.mockRestore();
  });

  it('asks for no password: the email and password are typed on the hosted page', () => {
    const view = render(<LoginScreen />);

    expect(view.queryByTestId('email-input')).toBeNull();
    expect(view.queryByTestId('password-input')).toBeNull();
    expect(view.getByTestId('login-button')).toBeTruthy();
    // The other ways in stay: forgot password and registration.
    expect(view.getByTestId('forgot-password-link')).toBeTruthy();
    expect(view.getByText(i18n.t('app.createOne'))).toBeTruthy();
  });

  it('starts the hosted sign-in from the button and shows nothing once it succeeds', async () => {
    mockSignIn.mockResolvedValue(true);
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(mockSignIn).toHaveBeenCalledTimes(1));
    expect(alertSpy).not.toHaveBeenCalled();
  });

  it('says nothing when the athlete closes the browser', async () => {
    mockSignIn.mockResolvedValue(false);
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(mockSignIn).toHaveBeenCalledTimes(1));
    expect(alertSpy).not.toHaveBeenCalled();
  });

  it('names a suspended account, never the server’s English description', async () => {
    mockSignIn.mockRejectedValue(new SignInRefusedError('access_denied', 'Account suspended'));
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(alertSpy).toHaveBeenCalledWith(i18n.t('app.loginFailedTitle'), i18n.t('shell.accountSuspendedBody'));
  });

  it('reads a callback that was not this sign-in’s as a failed sign-in', async () => {
    mockSignIn.mockRejectedValue(new SignInRefusedError('state_mismatch'));
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(alertSpy).toHaveBeenCalledWith(i18n.t('app.loginFailedTitle'), i18n.t('auth.loginFailed'));
  });

  it('reads a code that would not redeem as a failed sign-in, not as a wrong password', async () => {
    mockSignIn.mockRejectedValue(
      Object.assign(new Error('Request failed with status code 400'), {
        isAxiosError: true,
        response: { status: 400, data: { error: 'invalid_grant' } },
      }),
    );
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(alertSpy).toHaveBeenCalledWith(i18n.t('app.loginFailedTitle'), i18n.t('auth.loginFailed'));
    expect(alertSpy).not.toHaveBeenCalledWith(expect.anything(), i18n.t('auth.invalidCredentials'));
  });

  it('names an offline phone as one', async () => {
    mockSignIn.mockRejectedValue(Object.assign(new Error('Network Error'), { code: 'ERR_NETWORK', isAxiosError: true }));
    const view = render(<LoginScreen />);

    fireEvent.press(view.getByTestId('login-button'));

    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(alertSpy).toHaveBeenCalledWith(i18n.t('app.loginFailedTitle'), i18n.t('errors.network'));
  });
});
