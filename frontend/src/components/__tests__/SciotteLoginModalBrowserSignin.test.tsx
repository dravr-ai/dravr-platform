// ABOUTME: carnet#1617 — with the Dravr extension, a provider sign-in runs in the athlete's own browser and its cookies connect it
// ABOUTME: Pins the button only the extension shows, the hand-over that connects, and the closed tab that returns to the credentials

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const browserSigninCatalog = vi.fn();
const browserSignin = vi.fn();
const detectSigninExtension = vi.fn();
const signInWithExtension = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: vi.fn(),
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

function renderModal(onConnected = vi.fn()) {
  const view = render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target="garmin" />
    </ThemeProvider>,
  );
  return { onConnected, unmount: view.unmount };
}

describe('SciotteLoginModal — a sign-in in the athlete\'s own browser', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    browserSigninCatalog.mockResolvedValue({ providers: [GARMIN] });
  });

  it('offers no browser sign-in without the extension', async () => {
    detectSigninExtension.mockResolvedValue(null);
    renderModal();
    await screen.findByLabelText(/email/i);
    expect(screen.queryByRole('button', { name: /Garmin/ })).toBeNull();
  });

  it('connects the provider with the cookies the extension hands over', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    signInWithExtension.mockResolvedValue(COOKIES);
    browserSignin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    const { onConnected } = renderModal();

    await userEvent.setup().click(await screen.findByRole('button', { name: /Garmin/ }));

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(signInWithExtension).toHaveBeenCalledWith(GARMIN, expect.any(AbortSignal));
    expect(browserSignin).toHaveBeenCalledWith('garmin', COOKIES, { tosConsent: false });
  });

  it('stops looking for the extension when the window goes away', async () => {
    detectSigninExtension.mockReturnValue(new Promise(() => {}));
    const { unmount } = renderModal();
    const [signal] = detectSigninExtension.mock.calls[0] as [AbortSignal];
    expect(signal.aborted).toBe(false);

    unmount();

    expect(signal.aborted).toBe(true);
  });

  it('abandons a sign-in in progress when the window goes away', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    signInWithExtension.mockReturnValue(new Promise(() => {}));
    const { unmount } = renderModal();
    await userEvent.setup().click(await screen.findByRole('button', { name: /Garmin/ }));
    await vi.waitFor(() => expect(signInWithExtension).toHaveBeenCalledTimes(1));
    const [, signal] = signInWithExtension.mock.calls[0] as [unknown, AbortSignal];

    unmount();

    expect(signal.aborted).toBe(true);
  });

  it('returns to the credentials when the athlete does not accept the extension\'s notice', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    const { ExtensionSignInError } = await import('../../utils/signinExtension');
    signInWithExtension.mockRejectedValue(new ExtensionSignInError('declined'));
    const { onConnected } = renderModal();

    await userEvent.setup().click(await screen.findByRole('button', { name: /Garmin/ }));

    await screen.findByLabelText(/email/i);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(onConnected).not.toHaveBeenCalled();
  });

  it('returns to the credentials when the athlete closes the sign-in tab', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    const { ExtensionSignInError } = await import('../../utils/signinExtension');
    signInWithExtension.mockRejectedValue(new ExtensionSignInError('closed'));
    const { onConnected } = renderModal();

    await userEvent.setup().click(await screen.findByRole('button', { name: /Garmin/ }));

    await screen.findByLabelText(/email/i);
    expect(browserSignin).not.toHaveBeenCalled();
    expect(onConnected).not.toHaveBeenCalled();
  });
});
