// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The page's side of the extension protocol, over the content script's window channel (carnet#1617)
// ABOUTME: A fake content script answers on the page's own origin; detection, sign-in, failure, closing and keepalive are pinned

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  ExtensionSignInError,
  SIGNIN_EXTENSION_CHANNEL,
  detectSigninExtension,
  signInWithExtension,
} from '../signinExtension';

const GARMIN = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  success_patterns: ['/app'],
  cookie_domain: 'garmin.com',
};
const COOKIES = [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }];

interface PageMessage {
  channel: string;
  to: string;
  session: string;
  kind: 'open' | 'message' | 'close';
  payload?: { type: string };
}

/**
 * A fake bridge.js: it reads what the page posts to the extension and answers
 * as the content script would, through `answer`, on the page's own origin.
 */
function fakeBridge(answer: (message: PageMessage, reply: (kind: string, payload?: unknown) => void) => void) {
  const seen: PageMessage[] = [];
  const listener = (event: MessageEvent) => {
    const data = event.data as PageMessage;
    if (data?.channel !== SIGNIN_EXTENSION_CHANNEL || data.to !== 'extension') return;
    seen.push(data);
    answer(data, (kind, payload) =>
      window.dispatchEvent(
        new MessageEvent('message', {
          data: { channel: SIGNIN_EXTENSION_CHANNEL, to: 'page', session: data.session, kind, payload },
          origin: window.location.origin,
          source: window,
        }),
      ),
    );
  };
  window.addEventListener('message', listener);
  return { seen, stop: () => window.removeEventListener('message', listener) };
}

/** window.postMessage dispatches through the event loop in jsdom; deliver it as a browser would, same origin. */
beforeEach(() => {
  vi.spyOn(window, 'postMessage').mockImplementation((data: unknown) => {
    window.dispatchEvent(new MessageEvent('message', { data, origin: window.location.origin, source: window }));
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('detectSigninExtension', () => {
  it('reads the extension as absent when nothing answers', async () => {
    vi.useFakeTimers();
    const detected = detectSigninExtension();
    await vi.advanceTimersByTimeAsync(1_000);
    await expect(detected).resolves.toBeNull();
  });

  it('answers the installed version', async () => {
    const bridge = fakeBridge((message, reply) => {
      if (message.kind === 'open') reply('opened');
      if (message.kind === 'message' && message.payload?.type === 'ping') reply('message', { type: 'pong', version: '0.1.0' });
    });
    await expect(detectSigninExtension()).resolves.toBe('0.1.0');
    expect(bridge.seen.map((message) => message.kind)).toEqual(['open', 'message', 'close']);
    bridge.stop();
  });

  it('ignores an answer from another origin', async () => {
    vi.useFakeTimers();
    const listener = (event: MessageEvent) => {
      const data = event.data as PageMessage;
      if (data?.to !== 'extension' || data.kind !== 'message') return;
      window.dispatchEvent(
        new MessageEvent('message', {
          data: { channel: SIGNIN_EXTENSION_CHANNEL, to: 'page', session: data.session, kind: 'message', payload: { type: 'pong', version: '9' } },
          origin: 'https://evil.test',
          source: window,
        }),
      );
    };
    window.addEventListener('message', listener);
    const detected = detectSigninExtension();
    await vi.advanceTimersByTimeAsync(1_000);
    await expect(detected).resolves.toBeNull();
    window.removeEventListener('message', listener);
  });
});

describe('signInWithExtension', () => {
  it('answers the cookies the extension hands over, having asked for the provider entry', async () => {
    const bridge = fakeBridge((message, reply) => {
      if (message.kind === 'open') reply('opened');
      if (message.kind === 'message' && message.payload?.type === 'signin') {
        reply('message', { type: 'signed_in', provider: 'garmin', cookies: COOKIES });
      }
    });
    await expect(signInWithExtension(GARMIN)).resolves.toEqual(COOKIES);
    expect(bridge.seen[1].payload).toEqual({
      type: 'signin',
      provider: 'garmin',
      loginUrl: GARMIN.login_url,
      successPatterns: GARMIN.success_patterns,
      cookieDomain: 'garmin.com',
    });
    bridge.stop();
  });

  it('rejects with the reason the extension gives', async () => {
    const bridge = fakeBridge((message, reply) => {
      if (message.kind === 'open') reply('opened');
      if (message.kind === 'message') reply('message', { type: 'failed', reason: 'closed' });
    });
    await expect(signInWithExtension(GARMIN)).rejects.toEqual(new ExtensionSignInError('closed'));
    bridge.stop();
  });

  it('rejects when the extension closes the exchange', async () => {
    const bridge = fakeBridge((message, reply) => {
      if (message.kind === 'open') reply('opened');
      if (message.kind === 'message') reply('closed');
    });
    await expect(signInWithExtension(GARMIN)).rejects.toMatchObject({ reason: 'disconnected' });
    bridge.stop();
  });

  it('keeps the extension awake while the athlete signs in', async () => {
    vi.useFakeTimers();
    const bridge = fakeBridge((message, reply) => {
      if (message.kind === 'open') reply('opened');
    });
    const signing = signInWithExtension(GARMIN);
    await vi.advanceTimersByTimeAsync(45_000);
    const keepalives = bridge.seen.filter((message) => message.payload?.type === 'keepalive');
    expect(keepalives.length).toBe(2);
    bridge.stop();
    signing.catch(() => {});
  });
});
