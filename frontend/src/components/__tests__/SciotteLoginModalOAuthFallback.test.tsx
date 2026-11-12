// ABOUTME: A sign-in-first card's account sign-in that cannot complete hands over to the provider's OAuth (carnet#1477)
// ABOUTME: Pins the reasons that fall back, the wrong credentials that never do, and a window offered no fallback

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    sciotteCancel: vi.fn().mockResolvedValue({ cancelled: false }),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(onFallBackToOAuth?: () => void) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal
        isOpen
        onClose={vi.fn()}
        onConnected={vi.fn()}
        target="garmin"
        onFallBackToOAuth={onFallBackToOAuth}
      />
    </ThemeProvider>,
  );
}

async function signIn() {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText(/email/i), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

const refusal = (reason: string) => ({
  response: { status: 400, data: { code: 'InvalidInput', message: 'x', details: { reason } } },
});

describe('SciotteLoginModal — the OAuth a failed sign-in falls back to', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  for (const reason of ['sign_in_unavailable', 'login_challenge']) {
    it(`hands a ${reason} refusal to the provider's OAuth instead of showing it`, async () => {
      sciotteLogin.mockRejectedValue(refusal(reason));
      const onFallBackToOAuth = vi.fn();
      renderModal(onFallBackToOAuth);

      await signIn();

      await vi.waitFor(() => expect(onFallBackToOAuth).toHaveBeenCalledTimes(1));
      expect(screen.queryByRole('button', { name: /try again|retry/i })).not.toBeInTheDocument();
    });
  }

  it('keeps wrong credentials in the window, for the athlete to type again', async () => {
    sciotteLogin.mockResolvedValue({ status: 'failed', error: 'Invalid email or password' });
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });

  it('keeps any other refusal in the window', async () => {
    sciotteLogin.mockRejectedValue(refusal('login_flow_expired'));
    const onFallBackToOAuth = vi.fn();
    renderModal(onFallBackToOAuth);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry|start again/i })).toBeInTheDocument();
    expect(onFallBackToOAuth).not.toHaveBeenCalled();
  });

  it('shows the failure when its card offers no OAuth to fall back to', async () => {
    sciotteLogin.mockRejectedValue(refusal('sign_in_unavailable'));
    renderModal(undefined);

    await signIn();

    expect(await screen.findByRole('button', { name: /try again|retry/i })).toBeInTheDocument();
  });
});
