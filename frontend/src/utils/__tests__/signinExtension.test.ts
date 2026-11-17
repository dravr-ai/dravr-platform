// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the page's side of the sign-in extension's port: detecting it, and a sign-in's answer or failure
// ABOUTME: A stand-in chrome.runtime plays the extension (the test double this module's port protocol needs; carnet#1617)

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  detectSigninExtension,
  ExtensionSignInError,
  SIGNIN_EXTENSION_ID,
  signInWithExtension,
} from '../signinExtension';

type Listener = (message: unknown) => void;

/**
 * A port the stand-in extension answers through: `answer` replies to each
 * message the page posts, and `disconnect` from the extension's side fires the
 * page's `onDisconnect`.
 */
class FakePort {
  readonly posted: unknown[] = [];
  disconnected = false;
  private readonly messageListeners: Listener[] = [];
  private readonly disconnectListeners: Array<() => void> = [];

  constructor(private readonly answer: (message: unknown, port: FakePort) => void) {}

  onMessage = { addListener: (listener: Listener) => void this.messageListeners.push(listener) };
  onDisconnect = { addListener: (listener: () => void) => void this.disconnectListeners.push(listener) };

  postMessage(message: unknown): void {
    this.posted.push(message);
    this.answer(message, this);
  }

  disconnect(): void {
    this.disconnected = true;
  }

  send(message: unknown): void {
    for (const listener of this.messageListeners) listener(message);
  }

  dropFromExtension(): void {
    for (const listener of this.disconnectListeners) listener();
  }
}

/** Install a `chrome.runtime` whose ports `answer` each message. */
function installExtension(answer: (message: unknown, port: FakePort) => void): { ports: FakePort[]; ids: string[] } {
  const ports: FakePort[] = [];
  const ids: string[] = [];
  vi.stubGlobal('chrome', {
    runtime: {
      connect: (id: string) => {
        ids.push(id);
        const port = new FakePort(answer);
        ports.push(port);
        return port;
      },
    },
  });
  return { ports, ids };
}

const GARMIN = {
  provider: 'garmin',
  login_url: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  success_patterns: ['/app'],
  cookie_domain: 'garmin.com',
};
const COOKIES = [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }];

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('detectSigninExtension', () => {
  it('is null when no extension admits this page', async () => {
    await expect(detectSigninExtension()).resolves.toBeNull();
  });

  it("is the extension's version when it answers the ping, and closes the port", async () => {
    const { ports, ids } = installExtension((message, port) => {
      if ((message as { type: string }).type === 'ping') port.send({ type: 'pong', version: '0.1.0' });
    });
    await expect(detectSigninExtension()).resolves.toBe('0.1.0');
    expect(ids).toEqual([SIGNIN_EXTENSION_ID]);
    expect(ports[0].disconnected).toBe(true);
  });

  it('is null when nothing answers before the ping times out', async () => {
    vi.useFakeTimers();
    installExtension(() => {});
    const detected = detectSigninExtension();
    await vi.advanceTimersByTimeAsync(1000);
    await expect(detected).resolves.toBeNull();
  });
});

describe('signInWithExtension', () => {
  it("asks for the entry's sign-in and resolves with the cookies the extension hands back", async () => {
    const { ports } = installExtension((message, port) => {
      if ((message as { type: string }).type === 'signin') port.send({ type: 'signed_in', provider: 'garmin', cookies: COOKIES });
    });
    await expect(signInWithExtension(GARMIN)).resolves.toEqual(COOKIES);
    expect(ports[0].posted[0]).toEqual({
      type: 'signin',
      provider: 'garmin',
      loginUrl: GARMIN.login_url,
      successPatterns: GARMIN.success_patterns,
      cookieDomain: 'garmin.com',
    });
    expect(ports[0].disconnected).toBe(true);
  });

  it("rejects with the extension's reason when the sign-in fails", async () => {
    installExtension((message, port) => {
      if ((message as { type: string }).type === 'signin') port.send({ type: 'failed', reason: 'closed' });
    });
    const failure = signInWithExtension(GARMIN);
    await expect(failure).rejects.toBeInstanceOf(ExtensionSignInError);
    await expect(failure).rejects.toMatchObject({ reason: 'closed' });
  });

  it('rejects as disconnected when the extension drops the port', async () => {
    installExtension((message, port) => {
      if ((message as { type: string }).type === 'signin') port.dropFromExtension();
    });
    await expect(signInWithExtension(GARMIN)).rejects.toMatchObject({ reason: 'disconnected' });
  });

  it('keeps the extension awake while the athlete signs in, and stops once it ends', async () => {
    vi.useFakeTimers();
    const { ports } = installExtension(() => {});
    const signIn = signInWithExtension(GARMIN);
    await vi.advanceTimersByTimeAsync(45_000);
    const keepalives = () => ports[0].posted.filter((message) => (message as { type: string }).type === 'keepalive').length;
    expect(keepalives()).toBe(2);

    ports[0].send({ type: 'signed_in', provider: 'garmin', cookies: COOKIES });
    await expect(signIn).resolves.toEqual(COOKIES);
    await vi.advanceTimersByTimeAsync(60_000);
    expect(keepalives()).toBe(2);
  });

  it('rejects as not installed when no extension admits this page', async () => {
    await expect(signInWithExtension(GARMIN)).rejects.toMatchObject({ reason: 'not_installed' });
  });
});
