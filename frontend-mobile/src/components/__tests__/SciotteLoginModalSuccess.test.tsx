// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The mobile credential login's success state names the platform in one sentence, in the app language; history mode says the copy runs
// ABOUTME: A refused sign-in says so in the open window, which stays open for another try

import React from 'react';
import { render, screen, fireEvent, waitFor } from '@testing-library/react-native';
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
    sciotteCancel: jest.fn().mockResolvedValue({ cancelled: false }),
    initMobileOAuth: jest.fn(),
  },
}));
jest.mock('../OAuthAppSetupModal', () => ({ OAuthAppSetupModal: () => null }));

const sciotteLogin = oauthApi.sciotteLogin as jest.Mock;

async function connectGarmin(props: Partial<React.ComponentProps<typeof SciotteLoginModal>> = {}): Promise<void> {
  render(
    <SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" {...props} />,
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

describe('SciotteLoginModal (mobile) — history mode', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'connected' });
  });

  it('says the sign-in copies the history once, and that the copy runs', async () => {
    await connectGarmin({ historyMode: true });

    expect(await screen.findByTestId('sciotte-success-line')).toHaveTextContent(i18n.t('providers.history.modalCopying'));
    expect(screen.queryByText('Garmin is ready to sync')).toBeNull();
  });

  it('says what the sign-in is for before it starts', () => {
    render(<SciotteLoginModal visible historyMode onClose={jest.fn()} onConnected={jest.fn()} target="garmin" />);

    expect(screen.getByTestId('sciotte-history-intro')).toHaveTextContent(
      i18n.t('providers.history.modalIntro', { provider: 'Garmin' }),
    );
  });

  it('says nothing of history on an ordinary sign-in', () => {
    render(<SciotteLoginModal visible onClose={jest.fn()} onConnected={jest.fn()} target="garmin" />);

    expect(screen.queryByTestId('sciotte-history-intro')).toBeNull();
  });
});

describe('SciotteLoginModal (mobile) — a refused Garmin sign-in', () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it('shows the refusal in the window and stays open', async () => {
    sciotteLogin.mockRejectedValue({ response: { status: 401, data: { message: 'bad password' } } });
    const onClose = jest.fn();

    await connectGarmin({ onClose });

    expect(await screen.findByText(i18n.t('app.connectionFailed'))).toBeTruthy();
    expect(onClose).not.toHaveBeenCalled();
  });
});
