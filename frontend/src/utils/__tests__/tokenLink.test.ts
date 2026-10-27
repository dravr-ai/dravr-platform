// ABOUTME: Tests the mailed links' token parsing, and that their fragment is never taken for a dashboard deep link
// ABOUTME: The deep-link read happens at module load, so each case sets the address bar and re-imports sessionRoute
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { afterEach, describe, expect, it, vi } from 'vitest';
import { ACCEPT_INVITATION_PATH, VERIFY_EMAIL_PATH, parseLinkToken } from '../tokenLink';

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234';

describe('parseLinkToken', () => {
  it('reads the token from the link fragment', () => {
    expect(parseLinkToken(`#token=${TOKEN}`)).toBe(TOKEN);
  });

  it('answers null for a fragment with no token', () => {
    expect(parseLinkToken('')).toBeNull();
    expect(parseLinkToken('#token=')).toBeNull();
    expect(parseLinkToken('#home')).toBeNull();
  });
});

describe('a token fragment and the dashboard deep link', () => {
  afterEach(() => {
    window.history.replaceState(null, '', '/');
  });

  async function deepLinkAfterLoading(url: string): Promise<string | null> {
    window.history.replaceState(null, '', url);
    vi.resetModules();
    const { peekDeepLink } = await import('../sessionRoute');
    return peekDeepLink();
  }

  it('does not hold the invitation token as a link to reopen after sign-in', async () => {
    expect(await deepLinkAfterLoading(`${ACCEPT_INVITATION_PATH}#token=${TOKEN}`)).toBeNull();
  });

  it('does not hold the confirmation token as a link to reopen after sign-in', async () => {
    expect(await deepLinkAfterLoading(`${VERIFY_EMAIL_PATH}#token=${TOKEN}`)).toBeNull();
  });

  it('still holds a followed dashboard link on any other page', async () => {
    expect(await deepLinkAfterLoading('/#coaches')).toBe('coaches');
  });
});
