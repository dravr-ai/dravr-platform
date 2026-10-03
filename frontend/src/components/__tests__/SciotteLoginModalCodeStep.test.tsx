// ABOUTME: A code the provider refuses keeps the sign-in on its code step, with an inline message and an empty, focused input
// ABOUTME: Pins the "Verifying code" spinner, the lapsed and failed sign-in copy, and the code input's accessibility
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();
const sciotteSubmitOTP = vi.fn();
const sciotteSelect2FA = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteConfig: vi.fn().mockResolvedValue({ login_timeout_secs: 240 }),
    sciotteSelect2FA: (...args: unknown[]) => sciotteSelect2FA(...args),
    sciotteSubmitOTP: (...args: unknown[]) => sciotteSubmitOTP(...args),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

const CODE_LABEL = 'Enter the 6-digit code COROS sent you';

function renderModal(onConnected = vi.fn(), target: 'coros' | 'strava' = 'coros') {
  render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={onConnected} target={target} consentRequired={false} />
    </ThemeProvider>,
  );
  return onConnected;
}

/** Sign in up to COROS's code step. */
async function reachCodeStep(codeLabel = CODE_LABEL) {
  const user = userEvent.setup();
  await signIn(user);
  await screen.findByLabelText(codeLabel);
  return user;
}

/** Type the credentials and log in. */
async function signIn(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText('Email'), 'athlete@example.com');
  await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
  await user.click(screen.getByRole('button', { name: 'Log In' }));
}

async function submitCode(user: ReturnType<typeof userEvent.setup>, code: string) {
  await user.type(screen.getByLabelText(CODE_LABEL), code);
  await user.click(screen.getByRole('button', { name: 'Verify' }));
}

describe('SciotteLoginModal — the code step', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sciotteLogin.mockResolvedValue({ status: 'otp_required' });
  });

  it('keeps the sign-in on the code step when a code is refused, then connects on the right one', async () => {
    const onConnected = renderModal();
    const user = await reachCodeStep();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();

    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'otp_required', reason: 'code_rejected' });
    await submitCode(user, '000000');

    expect(await screen.findByRole('alert')).toHaveTextContent(
      "That code wasn't accepted. Check it and enter it again.",
    );
    const input = screen.getByLabelText(CODE_LABEL);
    expect(input).toHaveValue('');
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(input).toHaveAccessibleDescription("That code wasn't accepted. Check it and enter it again.");

    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'connected', provider: 'sciotte_coros' });
    await submitCode(user, '246810');

    expect(await screen.findByText('Your COROS data is now available')).toBeInTheDocument();
    expect(onConnected).toHaveBeenCalledTimes(1);
    expect(sciotteLogin).toHaveBeenCalledTimes(1);
    expect(sciotteSubmitOTP.mock.calls).toEqual([['000000'], ['246810']]);
  });

  it('says it is verifying the code while the code is checked', async () => {
    renderModal();
    const user = await reachCodeStep();
    sciotteSubmitOTP.mockReturnValueOnce(new Promise(() => {}));
    await submitCode(user, '246810');

    expect(await screen.findAllByText('Verifying code...')).not.toHaveLength(0);
    expect(screen.queryByText(/Submitting your credentials/)).not.toBeInTheDocument();
    expect(screen.queryByText(/may take up to/i)).not.toBeInTheDocument();
  });

  it('asks to start again when the sign-in lapsed', async () => {
    renderModal();
    const user = await reachCodeStep();
    sciotteSubmitOTP.mockRejectedValueOnce({
      response: { status: 400, data: { details: { reason: 'login_flow_expired' } } },
    });
    await submitCode(user, '246810');

    expect(
      await screen.findByText('This sign-in is no longer active. Please start the sign-in again.'),
    ).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: 'Try Again' }));
    expect(screen.getByLabelText('Email')).toBeInTheDocument();
  });

  it('words a failed code in the app, not in the provider prose', async () => {
    renderModal();
    const user = await reachCodeStep();
    sciotteSubmitOTP.mockResolvedValueOnce({ status: 'failed', error: 'Too many attempts' });
    await submitCode(user, '246810');

    expect(await screen.findByText('That code did not work. Sign in again to get a new code.')).toBeInTheDocument();
    expect(screen.queryByText('Too many attempts')).not.toBeInTheDocument();
  });

  it('labels a six-digit numeric one-time-code input', async () => {
    renderModal();
    await reachCodeStep();
    const input = screen.getByLabelText(CODE_LABEL);
    expect(input).toHaveAttribute('maxLength', '6');
    expect(input).toHaveAttribute('inputMode', 'numeric');
    expect(input).toHaveAttribute('autoComplete', 'one-time-code');
    expect(input).not.toHaveAttribute('aria-invalid');
  });

  it('names no sender on a code step behind a Google sign-in', async () => {
    renderModal(vi.fn(), 'strava');
    const user = userEvent.setup();
    await user.click(screen.getByRole('button', { name: 'Continue with Google' }));
    await signIn(user);

    expect(await screen.findByLabelText('Enter the code sent to your device')).toBeInTheDocument();
    expect(screen.queryByText(/Strava sent you/)).not.toBeInTheDocument();
  });

  it('words a failed 2FA pick in the app, not in the provider prose', async () => {
    sciotteLogin.mockResolvedValueOnce({
      status: 'two_factor_choice',
      options: [{ id: 'sms', label: 'Text message' }],
    });
    renderModal();
    const user = userEvent.setup();
    await signIn(user);
    sciotteSelect2FA.mockResolvedValueOnce({ status: 'failed', error: 'Too many attempts' });
    await user.click(await screen.findByRole('button', { name: 'Text message' }));

    expect(await screen.findByText('Verification failed')).toBeInTheDocument();
    expect(screen.queryByText('Too many attempts')).not.toBeInTheDocument();
  });
});
