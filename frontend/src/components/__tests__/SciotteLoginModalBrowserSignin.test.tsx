// ABOUTME: carnet#1617 — with the Dravr extension, a provider sign-in runs in the athlete's own browser and its cookies connect it
// ABOUTME: Pins the extension going first, its own button beside the credentials, the hand-over, and the closed tab that returns to the form

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';
import type { SciotteTarget } from '@pierre/shared-types';

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
const SIGN_IN_UNAVAILABLE_REFUSAL = {
  response: { status: 400, data: { code: 'InvalidInput', message: 'x', details: { reason: 'sign_in_unavailable' } } },
};
const COOKIES = [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }];

// The extension's button, by its own label: it names the provider's page and
// never reads « Continue with … », the OAuth buttons' label (carnet#1617).
const EXTENSION_BUTTON = { name: /Sign in on Garmin's page/ };

interface RenderOptions {
  onConnected?: () => void;
  extensionFirst?: boolean;
  target?: SciotteTarget;
  consentRequired?: boolean;
  onFallBackToOAuth?: () => void;
}

function renderModal({
  onConnected = vi.fn(),
  extensionFirst = false,
  target = 'garmin',
  consentRequired = false,
  onFallBackToOAuth,
}: RenderOptions = {}) {
  const view = render(
    <ThemeProvider>
      <SciotteLoginModal
        isOpen
        onClose={vi.fn()}
        onConnected={onConnected}
        target={target}
        consentRequired={consentRequired}
        extensionFirst={extensionFirst}
        onFallBackToOAuth={onFallBackToOAuth}
      />
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
    expect(screen.queryByRole('button', EXTENSION_BUTTON)).toBeNull();
  });

  it('connects the provider with the cookies the extension hands over', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    signInWithExtension.mockResolvedValue(COOKIES);
    browserSignin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    const { onConnected } = renderModal();

    await userEvent.setup().click(await screen.findByRole('button', EXTENSION_BUTTON));

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(signInWithExtension).toHaveBeenCalledWith(GARMIN, expect.any(AbortSignal));
    expect(browserSignin).toHaveBeenCalledWith('garmin', COOKIES, {
      tosConsent: false,
      signal: expect.any(AbortSignal),
    });
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
    await userEvent.setup().click(await screen.findByRole('button', EXTENSION_BUTTON));
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

    await userEvent.setup().click(await screen.findByRole('button', EXTENSION_BUTTON));

    await screen.findByLabelText(/email/i);
    expect(screen.queryByRole('alert')).toBeNull();
    expect(onConnected).not.toHaveBeenCalled();
  });

  it('returns to the credentials when the athlete closes the sign-in tab', async () => {
    detectSigninExtension.mockResolvedValue('0.1.0');
    const { ExtensionSignInError } = await import('../../utils/signinExtension');
    signInWithExtension.mockRejectedValue(new ExtensionSignInError('closed'));
    const { onConnected } = renderModal();

    await userEvent.setup().click(await screen.findByRole('button', EXTENSION_BUTTON));

    await screen.findByLabelText(/email/i);
    expect(browserSignin).not.toHaveBeenCalled();
    expect(onConnected).not.toHaveBeenCalled();
  });

  it('starts the extension as the window opens when the card puts it first', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    signInWithExtension.mockResolvedValue(COOKIES);
    browserSignin.mockResolvedValue({ status: 'connected', provider: 'sciotte_garmin' });
    const { onConnected } = renderModal({ extensionFirst: true });

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(signInWithExtension).toHaveBeenCalledTimes(1);
    expect(browserSignin).toHaveBeenCalledWith('garmin', COOKIES, {
      tosConsent: false,
      signal: expect.any(AbortSignal),
    });
  });

  it('waits for the click when the card puts the credentials first', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    renderModal({ extensionFirst: false });

    await screen.findByRole('button', EXTENSION_BUTTON);
    expect(signInWithExtension).not.toHaveBeenCalled();
  });

  it('leaves the form, with the extension still offered, once its tab is closed — and does not restart it', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    const { ExtensionSignInError } = await import('../../utils/signinExtension');
    signInWithExtension.mockRejectedValue(new ExtensionSignInError('closed'));
    renderModal({ extensionFirst: true });

    await vi.waitFor(() => expect(signInWithExtension).toHaveBeenCalledTimes(1));
    await screen.findByLabelText(/email/i);
    expect(screen.getByRole('button', EXTENSION_BUTTON)).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Continue with Garmin/ })).toBeNull();
    expect(signInWithExtension).toHaveBeenCalledTimes(1);
  });

  it('offers the form, never the OAuth, when the extension sign-in fails', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    signInWithExtension.mockResolvedValue(COOKIES);
    // The refusal a credentials sign-in falls back to the OAuth on: the card
    // offers its OAuth, so only the order (extension, credentials, OAuth)
    // keeps it off this failure.
    browserSignin.mockRejectedValue(SIGN_IN_UNAVAILABLE_REFUSAL);
    const onFallBackToOAuth = vi.fn();
    renderModal({ extensionFirst: true, onFallBackToOAuth });

    const tryAgain = await screen.findByRole('button', { name: /try again/i });
    expect(screen.queryByTestId('sciotte-oauth-fallback')).toBeNull();
    await userEvent.setup().click(tryAgain);

    await screen.findByLabelText(/email/i);
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
    expect(signInWithExtension).toHaveBeenCalledTimes(1);
  });

  it('starts the extension only once the notice is accepted, and hands that consent over', async () => {
    detectSigninExtension.mockResolvedValue('0.2.0');
    browserSigninCatalog.mockResolvedValue({ providers: [{ ...GARMIN, provider: 'trainingpeaks' }] });
    signInWithExtension.mockResolvedValue(COOKIES);
    browserSignin.mockResolvedValue({ status: 'connected', provider: 'sciotte_trainingpeaks' });
    const { onConnected } = renderModal({ extensionFirst: true, target: 'trainingpeaks', consentRequired: true });

    expect(await screen.findByTestId('sciotte-extension-signin')).toBeDisabled();
    expect(signInWithExtension).not.toHaveBeenCalled();

    await userEvent.setup().click(
      screen.getByLabelText('I understand that TrainingPeaks could suspend my account, and I accept that risk.'),
    );

    await vi.waitFor(() => expect(onConnected).toHaveBeenCalledTimes(1));
    expect(signInWithExtension).toHaveBeenCalledTimes(1);
    expect(browserSignin).toHaveBeenCalledWith('trainingpeaks', COOKIES, {
      tosConsent: true,
      signal: expect.any(AbortSignal),
    });
  });
});
