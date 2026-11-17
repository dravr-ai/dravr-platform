// ABOUTME: The extension's service worker: opens a provider's sign-in in a real tab and hands its cookies to the Dravr page that asked
// ABOUTME: Talks to the page over a port the page keeps alive, so a sign-in that takes minutes outlives the worker's idle timer (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { SIGN_IN_TIMEOUT_MS, isSignedIn, signInRequest, wireCookie } from './signin.js';

const VERSION = chrome.runtime.getManifest().version;

/**
 * A Dravr page (one `externally_connectable` admits) connects a port and
 * sends one message:
 * - `{type: 'ping'}`: answered `{type: 'pong', version}`, so the page knows
 *   the extension is installed.
 * - `{type: 'signin', provider, loginUrl, successPatterns, cookieDomain}`:
 *   the provider's sign-in opens in a tab of this browser. Once the tab lands
 *   signed in, the provider's cookies are read, the tab closes and the page
 *   receives `{type: 'signed_in', cookies}`; a tab closed first, a refused
 *   request or the timeout answers `{type: 'failed', reason}`.
 *
 * The page sends `{type: 'keepalive'}` every 20 s while it waits: a message
 * on the port keeps this service worker from being put to sleep mid sign-in.
 */
chrome.runtime.onConnectExternal.addListener((port) => {
  let sign = null;
  port.onMessage.addListener((message) => {
    if (message?.type === 'ping') {
      port.postMessage({ type: 'pong', version: VERSION });
      return;
    }
    if (message?.type === 'keepalive' || sign) return;
    const request = signInRequest(message);
    if (!request) {
      port.postMessage({ type: 'failed', reason: 'refused' });
      return;
    }
    sign = runSignIn(request, port);
  });
  port.onDisconnect.addListener(() => {
    sign?.abandon();
  });
});

/** Open `request`'s sign-in in a tab and answer `port` once it ends. */
function runSignIn(request, port) {
  let tabId = null;
  let done = false;

  const finish = (answer) => {
    if (done) return;
    done = true;
    chrome.tabs.onUpdated.removeListener(onUpdated);
    chrome.tabs.onRemoved.removeListener(onRemoved);
    clearTimeout(timer);
    try {
      port.postMessage(answer);
    } catch {
      // The page left before the answer: nothing to tell.
    }
  };

  const onUpdated = (id, change, tab) => {
    if (id !== tabId || done) return;
    const href = change.url ?? tab.url;
    if (!href || !isSignedIn(href, request)) return;
    chrome.cookies.getAll({ domain: request.cookieDomain }).then(
      (cookies) => {
        finish({ type: 'signed_in', provider: request.provider, cookies: cookies.map(wireCookie) });
        chrome.tabs.remove(id).catch(() => {});
      },
      () => finish({ type: 'failed', reason: 'cookies_unreadable' }),
    );
  };

  const onRemoved = (id) => {
    if (id === tabId) finish({ type: 'failed', reason: 'closed' });
  };

  const timer = setTimeout(() => {
    finish({ type: 'failed', reason: 'timeout' });
    if (tabId !== null) chrome.tabs.remove(tabId).catch(() => {});
  }, SIGN_IN_TIMEOUT_MS);

  chrome.tabs.onUpdated.addListener(onUpdated);
  chrome.tabs.onRemoved.addListener(onRemoved);
  chrome.tabs.create({ url: request.loginUrl, active: true }).then(
    (tab) => {
      tabId = tab.id;
      // Already signed in to the provider: the sign-in page redirects at once.
      if (tab.url && isSignedIn(tab.url, request)) onUpdated(tab.id, {}, tab);
    },
    () => finish({ type: 'failed', reason: 'tab_unopened' }),
  );

  return {
    abandon() {
      if (done) return;
      done = true;
      chrome.tabs.onUpdated.removeListener(onUpdated);
      chrome.tabs.onRemoved.removeListener(onRemoved);
      clearTimeout(timer);
    },
  };
}
