// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile credential login's success state names the platform in one sentence, in the app language
// ABOUTME: It used to put the platform name in front of an English "is ready to sync" whatever the language

import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
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

async function connectGarmin(): Promise<void> {
  render(
    <SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" />,
  );
  fireEvent.changeText(screen.getByTestId('sciotte-email'), 'athlete@example.test');
  fireEvent.changeText(screen.getByTestId('sciotte-password'), 'not-a-real-password');
  fireEvent.press(screen.getByTestId('sciotte-login-submit'));
}

describe('SciotteLoginModal (mobile) — connected', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'connected' });
  });

  it('says the platform is ready to sync, in English', async () => {
    await connectGarmin();

    expect(await screen.findByText('Garmin is ready to sync')).toBeTruthy();
  });

  it('says it as one French sentence', async () => {
    await i18n.changeLanguage('fr');
    try {
      await connectGarmin();

      expect(await screen.findByText('Tout est prêt pour synchroniser Garmin')).toBeTruthy();
    } finally {
      await i18n.changeLanguage('en');
    }
  });
});
