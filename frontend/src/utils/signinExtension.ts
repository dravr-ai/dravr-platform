// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Talks to the Dravr sign-in extension: whether it is installed, and a provider sign-in run in the athlete's own browser
// ABOUTME: Reaches it through its content script on this page (window.postMessage), the one channel Chrome and Firefox share (carnet#1617)

import type { BrowserSignin, BrowserSigninCookie } from '@pierre/shared-types';

/**
 * The channel the extension's content script (`extensions/dravr-signin/bridge.js`)
 * relays between this page and the extension.
 */
export const SIGNIN_EXTENSION_CHANNEL = 'dravr-signin';

/** How long the extension has to answer a ping before it counts as absent. */
const PING_TIMEOUT_MS = 800;

/** How often a waiting page tells the extension it is still there. */
const KEEPALIVE_MS = 20_000;

/** One exchange with the extension, through its content script. */
interface ExtensionPort {
  postMessage(message: unknown): void;
  disconnect(): void;
  onMessage: { addListener(listener: (message: ExtensionAnswer) => void): void };
  onDisconnect: { addListener(listener: () => void): void };
}

/** What the content script posts to this page. */
interface BridgeMessage {
  channel: typeof SIGNIN_EXTENSION_CHANNEL;
  to: 'page';
  session: string;
  kind: 'opened' | 'message' | 'closed';
  payload?: ExtensionAnswer;
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

function isBridgeMessage(data: unknown, session: string): data is BridgeMessage {
  if (typeof data !== 'object' || data === null) return false;
  const message = data as Partial<BridgeMessage>;
  return message.channel === SIGNIN_EXTENSION_CHANNEL && message.to === 'page' && message.session === session;
}

/**
 * Open an exchange with the extension: a session the content script maps to
 * a port into the extension. Nothing answers when the extension is not
 * installed; a port the content script has not confirmed within
 * `openTimeoutMs` reads as closed.
 */
function openPort(openTimeoutMs: number): ExtensionPort {
  const session = crypto.randomUUID();
  const onMessage: Array<(message: ExtensionAnswer) => void> = [];
  const onDisconnect: Array<() => void> = [];
  let open = true;
  const send = (kind: 'open' | 'message' | 'close', payload?: unknown) =>
    window.postMessage(
      { channel: SIGNIN_EXTENSION_CHANNEL, to: 'extension', session, kind, payload },
      window.location.origin,
    );
  const close = () => {
    if (!open) return;
    open = false;
    clearTimeout(unconfirmed);
    window.removeEventListener('message', listen);
  };
  const listen = (event: MessageEvent) => {
    if (event.source !== window || event.origin !== window.location.origin) return;
    if (!isBridgeMessage(event.data, session)) return;
    if (event.data.kind === 'opened') clearTimeout(unconfirmed);
    else if (event.data.kind === 'message' && event.data.payload) {
      const payload = event.data.payload;
      onMessage.forEach((listener) => listener(payload));
    } else if (event.data.kind === 'closed') {
      close();
      onDisconnect.forEach((listener) => listener());
    }
  };
  const unconfirmed = setTimeout(() => {
    close();
    onDisconnect.forEach((listener) => listener());
  }, openTimeoutMs);
  window.addEventListener('message', listen);
  send('open');
  return {
    postMessage: (message) => {
      if (open) send('message', message);
    },
    disconnect: () => {
      if (!open) return;
      send('close');
      close();
    },
    onMessage: { addListener: (listener) => onMessage.push(listener) },
    onDisconnect: { addListener: (listener) => onDisconnect.push(listener) },
  };
}

/** The installed extension's version, or `null` when it is not installed. */
export function detectSigninExtension(): Promise<string | null> {
  return new Promise((resolve) => {
    const port = openPort(PING_TIMEOUT_MS);
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
  return new Promise((resolve, reject) => {
    const port = openPort(PING_TIMEOUT_MS);
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
