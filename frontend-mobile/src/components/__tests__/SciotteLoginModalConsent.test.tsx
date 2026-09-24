// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile credential login states the TrainingPeaks notice before the credentials and holds Log In until accepted
// ABOUTME: Pins the login body each target sends: tos_consent only for a TrainingPeaks notice, a username for TrainingPeaks

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
import type { SciotteTarget } from '@pierre/shared-types';
import { SciotteLoginModal } from '../SciotteLoginModal';
import { oauthApi } from '../../services/api';

jest.mock('expo-linking', () => ({ parse: jest.fn(), createURL: jest.fn() }));
jest.mock('../../utils/oauth', () => ({ getOAuthCallbackUrl: () => 'dravr://oauth-callback' }));
jest.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: jest.fn(),
    sciotteSelect2FA: jest.fn(),
    sciotteSubmitOTP: jest.fn(),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;

function renderModal(target: SciotteTarget, consentRequired: boolean) {
  return render(
    <SciotteLoginModal
      visible
      onClose={jest.fn()}
      onConnected={jest.fn()}
      target={target}
      consentRequired={consentRequired}
    />,
  );
}

function fillCredentials(identifier: string) {
  fireEvent.changeText(screen.getByTestId('sciotte-email'), identifier);
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
}

describe('SciotteLoginModal (mobile) — the TrainingPeaks notice', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'stop here' });
  });

  it('states the notice before the credentials and sends nothing until it is accepted', async () => {
    renderModal('trainingpeaks', true);

    expect(screen.getByText('Before you connect TrainingPeaks')).toBeTruthy();
    expect(screen.getByText(/Terms of Use \(section 13\)/)).toBeTruthy();
    expect(
      screen.getByText(/athletes who confirm a link in a group you oversee/),
    ).toBeTruthy();
    expect(screen.getByText('Username')).toBeTruthy();

    fillCredentials('coach-account');
    const logIn = screen.getByTestId('sciotte-login-submit');
    expect(logIn).toBeDisabled();
    fireEvent.press(logIn);
    expect(sciotteLogin).not.toHaveBeenCalled();

    const consent = screen.getByTestId('sciotte-tos-consent');
    fireEvent.press(consent);
    expect(consent).toBeChecked();
    expect(logIn).not.toBeDisabled();
    fireEvent.press(logIn);

    await waitFor(() => expect(sciotteLogin).toHaveBeenCalledTimes(1));
    expect(sciotteLogin).toHaveBeenCalledWith({
      email: 'coach-account',
      password: 'not-a-real-password',
      method: 'email',
      target: 'trainingpeaks',
      tos_consent: true,
    });
  });

  it('goes straight to the credentials once the account has accepted the notice', async () => {
    renderModal('trainingpeaks', false);
    expect(screen.queryByTestId('sciotte-tos-notice')).toBeNull();

    fillCredentials('coach-account');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() =>
      expect(sciotteLogin).toHaveBeenCalledWith({
        email: 'coach-account',
        password: 'not-a-real-password',
        method: 'email',
        target: 'trainingpeaks',
      }),
    );
  });

  it('never shows the notice for Garmin, whatever the flag says', async () => {
    renderModal('garmin', true);
    expect(screen.queryByTestId('sciotte-tos-notice')).toBeNull();
    expect(screen.getByText('Garmin Account')).toBeTruthy();

    fillCredentials('athlete@example.test');
    fireEvent.press(screen.getByTestId('sciotte-login-submit'));

    await waitFor(() =>
      expect(sciotteLogin).toHaveBeenCalledWith({
        email: 'athlete@example.test',
        password: 'not-a-real-password',
        method: 'email',
        target: 'garmin',
      }),
    );
  });
});
