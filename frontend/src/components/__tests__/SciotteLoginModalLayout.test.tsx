// ABOUTME: The sign-in window keeps the show-password eye inside the field, on every branch that asks for a password
// ABOUTME: And while the login runs it says the wait may take a while, names no budget, and prints its progress line once
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { SciotteTarget } from '@pierre/shared-types';
import SciotteLoginModal from '../SciotteLoginModal';
import { ThemeProvider } from '../../hooks/useTheme';

const sciotteLogin = vi.fn();

vi.mock('../../services/api', () => ({
  oauthApi: {
    sciotteLogin: (...args: unknown[]) => sciotteLogin(...args),
    sciotteSelect2FA: vi.fn(),
    sciotteSubmitOTP: vi.fn(),
    authorizeUrl: vi.fn(),
  },
}));
vi.mock('../OAuthAppSetupModal', () => ({ default: () => null }));

function renderModal(target: SciotteTarget) {
  return render(
    <ThemeProvider>
      <SciotteLoginModal isOpen onClose={vi.fn()} onConnected={vi.fn()} target={target} consentRequired={false} />
    </ThemeProvider>,
  );
}

// The eye is drawn inside the field only when it is taken out of flow and
// pinned to the field wrapper. Without that it wrapped onto its own row under
// "Password" (carnet#1298) — jsdom has no layout, so the test pins the
// structure that produces the right one.
function expectEyeInsideField() {
  const eye = screen.getByRole('button', { name: 'Show password' });
  const wrapper = eye.parentElement as HTMLElement;
  expect(wrapper).toHaveClass('relative');
  expect(wrapper).toContainElement(screen.getByLabelText('Password'));
  expect(eye).toHaveClass('absolute', 'right-2', 'top-1/2', '-translate-y-1/2');
}

describe('SciotteLoginModal — the show-password eye', () => {
  it.each<SciotteTarget>(['garmin', 'trainingpeaks', 'coros'])('sits inside the %s password field', (target) => {
    renderModal(target);
    expectEyeInsideField();
  });

  it.each(['Continue with Google', 'Continue with Apple', 'Log in with Strava email'])(
    'sits inside the Strava password field after "%s"',
    async (method) => {
      const user = userEvent.setup();
      renderModal('strava');
      await user.click(screen.getByRole('button', { name: method }));
      expectEyeInsideField();
    },
  );
});

describe('SciotteLoginModal — while the login runs', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // The login never answers, so the window stays on its waiting screen.
    sciotteLogin.mockReturnValue(new Promise(() => {}));
  });

  async function startLogin() {
    const user = userEvent.setup();
    renderModal('garmin');
    await user.type(screen.getByLabelText('Email or username'), 'athlete@example.com');
    await user.type(screen.getByLabelText('Password'), 'not-a-real-password');
    await user.click(screen.getByRole('button', { name: 'Log In' }));
  }

  it('prints the progress line once, under the spinner, not again in the header', async () => {
    await startLogin();
    expect(screen.getAllByText('Preparing the connection…')).toHaveLength(1);
  });

  it('says the wait may take a while and names no budget', async () => {
    await startLogin();
    expect(screen.getByText('This may take a moment while we securely connect your account')).toBeInTheDocument();

    const elapsed = await screen.findByText(/^\d+s elapsed — this may take a while$/, undefined, { timeout: 3000 });
    expect(elapsed).not.toHaveTextContent(/up to|minute/);
  });
});
