// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Talks to the Dravr sign-in extension: whether it is installed, and a provider sign-in run in the athlete's own browser
// ABOUTME: The extension opens the provider's page in a real tab and answers with that provider's cookies (carnet#1617)

import type { BrowserSignin, BrowserSigninCookie } from '@pierre/shared-types';

/**
 * The extension's ID, fixed by the public key in its manifest
 * (`extensions/dravr-signin/manifest.json`), so a side-loaded copy and the
 * app agree on it.
 */
export const SIGNIN_EXTENSION_ID = 'hkcdkpfegfmnhlfialngmhfijbnpkkhf';

/** How long the extension has to answer a ping before it counts as absent. */
const PING_TIMEOUT_MS = 800;

/** How often a waiting page tells the extension it is still there. */
const KEEPALIVE_MS = 20_000;

/** The part of `chrome.runtime` a page reaches when an extension admits it. */
interface ExtensionPort {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: { addListener(listener: (message: ExtensionAnswer) => void): void };
  onDisconnect: { addListener(listener: () => void): void };
}

interface ChromeRuntime {
  connect(extensionId: string): ExtensionPort;
}

/** What the extension posts back on its port. */
type ExtensionAnswer =
  | { type: 'pong'; version: string }
  | { type: 'signed_in'; provider: string; cookies: BrowserSigninCookie[] }
  | { type: 'failed'; reason: string };

/** Why a sign-in through the extension ended without cookies. */
export class ExtensionSignInError extends Error {
  /** `closed`, `refused`, `timeout`, `disconnected`, `not_installed`, `unavailable`, … */
  readonly reason: string;

  constructor(reason: string) {
    super(`Browser sign-in ended: ${reason}`);
    this.name = 'ExtensionSignInError';
    this.reason = reason;
  }
}

/** `chrome.runtime`, present only when an installed extension admits this page. */
function runtime(): ChromeRuntime | null {
  const chrome = (window as Window & { chrome?: { runtime?: Partial<ChromeRuntime> } }).chrome;
  const connect = chrome?.runtime?.connect;
  return typeof connect === 'function' ? (chrome?.runtime as ChromeRuntime) : null;
}

/** The installed extension's version, or `null` when it is not installed. */
export function detectSigninExtension(): Promise<string | null> {
  const chromeRuntime = runtime();
  if (!chromeRuntime) return Promise.resolve(null);
  return new Promise((resolve) => {
    let port: ExtensionPort;
    try {
      port = chromeRuntime.connect(SIGNIN_EXTENSION_ID);
    } catch {
      resolve(null);
      return;
    }
    const timer = setTimeout(() => {
      port.disconnect();
      resolve(null);
    }, PING_TIMEOUT_MS);
    port.onMessage.addListener((answer) => {
      if (answer.type !== 'pong') return;
      clearTimeout(timer);
      port.disconnect();
      resolve(answer.version);
    });
    port.onDisconnect.addListener(() => {
      clearTimeout(timer);
      resolve(null);
    });
    port.postMessage({ type: 'ping' });
  });
}

/**
 * Run `entry`'s sign-in in a tab of the athlete's own browser, through the
 * extension, and answer the provider's cookies once the tab lands signed in.
 * Rejects with {@link ExtensionSignInError} when the athlete closes the tab,
 * the extension refuses the request or it times out.
 */
export function signInWithExtension(entry: BrowserSignin): Promise<BrowserSigninCookie[]> {
  const chromeRuntime = runtime();
  if (!chromeRuntime) return Promise.reject(new ExtensionSignInError('not_installed'));
  return new Promise((resolve, reject) => {
    const port = chromeRuntime.connect(SIGNIN_EXTENSION_ID);
    const keepalive = setInterval(() => port.postMessage({ type: 'keepalive' }), KEEPALIVE_MS);
    let settled = false;
    const settle = (finish: () => void) => {
      if (settled) return;
      settled = true;
      clearInterval(keepalive);
      port.disconnect();
      finish();
    };
    port.onMessage.addListener((answer) => {
      if (answer.type === 'signed_in') settle(() => resolve(answer.cookies));
      else if (answer.type === 'failed') settle(() => reject(new ExtensionSignInError(answer.reason)));
    });
    port.onDisconnect.addListener(() => {
      settle(() => reject(new ExtensionSignInError('disconnected')));
    });
    port.postMessage({
      type: 'signin',
      provider: entry.provider,
      loginUrl: entry.login_url,
      successPatterns: entry.success_patterns,
      cookieDomain: entry.cookie_domain,
    });
  });
}
