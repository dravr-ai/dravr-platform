// ABOUTME: Tests the invitation link's token parsing, and that its fragment is never taken for a dashboard deep link
// ABOUTME: The deep-link read happens at module load, so each case sets the address bar and re-imports sessionRoute
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { afterEach, describe, expect, it, vi } from 'vitest';
import { ACCEPT_INVITATION_PATH, parseInvitationToken } from '../invitationLink';

const TOKEN = 'AbCd1234EfGh5678.IjKl9012MnOp3456QrSt7890UvWx1234';

describe('parseInvitationToken', () => {
  it('reads the token from the link fragment', () => {
    expect(parseInvitationToken(`#token=${TOKEN}`)).toBe(TOKEN);
  });

  it('answers null for a fragment with no token', () => {
    expect(parseInvitationToken('')).toBeNull();
    expect(parseInvitationToken('#token=')).toBeNull();
    expect(parseInvitationToken('#home')).toBeNull();
  });
});

describe('the invitation fragment and the dashboard deep link', () => {
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

  it('still holds a followed dashboard link on any other page', async () => {
    expect(await deepLinkAfterLoading('/#coaches')).toBe('coaches');
  });
});
