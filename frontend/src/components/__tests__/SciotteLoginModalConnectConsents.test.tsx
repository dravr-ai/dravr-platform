// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Garmin sign-in carries the offered consents ticked before it opened (carnet#1157, carnet#1390)
// ABOUTME: Pins the credential login body and the browser-extension hand-over, with boxes ticked and with none

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const browserSigninCatalog = vi.fn();
const browserSignin = vi.fn();
const detectSigninExtension = vi.fn();
const signInWithExtension = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
    browserSigninCatalog: (...args: unknown[]) => browserSigninCatalog(...args),
    browserSignin: (...args: unknown[]) => browserSignin(...args),
  },
}));
vi.mock('../../utils/signinExtension', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../utils/signinExtension')>();
  return {
    ...actual,
    detectSigninExtension: (...args: unknown[]) => detectSigninExtension(...args),
    signInWithExtension: (...args: unknown[]) => signInWithExtension(...args),
  };
});
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

const GARMIN = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  success_patterns: ['/app'],
  cookie_domain: 'garmin.com',
};
const COOKIES = [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }];

function renderModal(consents?: readonly string[]) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={vi.fn()} target="garmin" consents={consents} />
    </ThemeProvider>,
  );
}

async function logIn() {
  const user = userEvent.setup();
  await user.type(await screen.findByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

describe('SciotteLoginModal — the consents a Garmin connect offers', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'stop here' });
    browserSigninCatalog.mockResolvedValue({ providers: [GARMIN] });
    detectSigninExtension.mockResolvedValue(null);
  });

  it('sends the boxes ticked with the credential login', async () => {
    renderModal(['garmin', 'garmin_workout_transfer']);
    await logIn();

    expect(sciotteLogin).toHaveBeenCalledTimes(1);
    expect(sciotteLogin.mock.calls[0][0]).toMatchObject({
      target: 'garmin',
      consents: ['garmin', 'garmin_workout_transfer'],
    });
  });

  it('sends no consent when none was ticked', async () => {
    renderModal([]);
    await logIn();

    expect(sciotteLogin).toHaveBeenCalledTimes(1);
    expect(sciotteLogin.mock.calls[0][0]).not.toHaveProperty('consents');
  });

  it('sends the boxes ticked with the browser-extension sign-in', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    signInWithExtension.mockResolvedValue(COOKIES);
    browserSignin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    renderModal(['garmin_workout_transfer']);

    await userEvent.setup().click(await screen.findByRole('button', { name: /Garmin/ }));

    await vi.waitFor(() =>
      expect(browserSignin).toHaveBeenCalledWith('garmin', COOKIES, {
        tosConsent: false,
        consents: ['garmin_workout_transfer'],
        signal: expect.any(AbortSignal),
      }),
    );
  });
});
