// ABOUTME: Tests for the extension's service worker: the port protocol, the sign-in tab, the cookie hand-over and its failures
// ABOUTME: A fake chrome.* stands in for the browser; runs under `bun test extensions/dravr-signin` (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { beforeEach, describe, expect, it } from 'bun:test';

/** A minimal event: listeners, and a way for the test to fire it. */
function event() {
  const listeners = new Set();
  return {
    addListener: (listener) => listeners.add(listener),
    removeListener: (listener) => listeners.delete(listener),
    fire: (...args) => [...listeners].forEach((listener) => listener(...args)),
    size: () => listeners.size,
  };
}

const NOTICE_URL = 'chrome-extension://dravr/notice.html';
const ACCEPTED = { noticeAccepted: { version: 1, acceptedAt: '2026-11-18T12:00:00.000Z' } };

/**
 * The fake browser the worker runs against, rebuilt for every test. Its
 * storage holds the notice's acceptance unless a test says otherwise; tabs
 * get ids from 7 up, in the order they open.
 */
function fakeChrome({ stored = ACCEPTED } = {}) {
  let nextTab = 7;
  const chrome = {
    runtime: {
      getManifest: () => ({ version: '0.1.0' }),
      getURL: (path) => `chrome-extension://dravr/${path}`,
      onConnect: event(),
      onInstalled: event(),
    },
    storage: {
      area: { ...stored },
      onChanged: event(),
      local: {
        get: async (key) => (key in chrome.storage.area ? { [key]: chrome.storage.area[key] } : {}),
      },
      /** What notice.js does when the athlete clicks I agree. */
      accept: () => {
        chrome.storage.area = { ...ACCEPTED };
        chrome.storage.onChanged.fire({ noticeAccepted: { newValue: ACCEPTED.noticeAccepted } }, 'local');
      },
    },
    tabs: {
      onUpdated: event(),
      onRemoved: event(),
      created: [],
      removed: [],
      create: async ({ url }) => {
        const tab = { id: nextTab++, url: 'about:blank', pendingUrl: url };
        chrome.tabs.created.push(url);
        return tab;
      },
      remove: async (id) => {
        chrome.tabs.removed.push(id);
      },
    },
    cookies: {
      jar: [],
      queried: [],
      getAll: async ({ domain }) => {
        chrome.cookies.queried.push(domain);
        return chrome.cookies.jar;
      },
    },
  };
  return chrome;
}

/**
 * The port bridge.js opens for the page: what the page posts in, what the
 * worker answers. By default it comes from a tab of Dravr's app.
 */
function connect(chrome, { name = 'dravr-signin', url = 'https://app.dravr.ai/settings' } = {}) {
  const answers = [];
  const port = {
    name,
    sender: { url, tab: { id: 3, url } },
    disconnected: false,
    left: false,
    onMessage: event(),
    onDisconnect: event(),
    // A port the page has left throws on a post, as the browser's does.
    postMessage: (message) => {
      if (port.left) throw new Error('Attempting to use a disconnected port object');
      answers.push(message);
    },
    disconnect: () => {
      port.disconnected = true;
    },
  };
  chrome.runtime.onConnect.fire(port);
  return {
    port,
    answers,
    send: (message) => port.onMessage.fire(message),
    /** The page goes away: the port disconnects and takes no more posts. */
    leave: () => {
      port.left = true;
      port.onDisconnect.fire();
    },
  };
}

/** A promise the test settles by hand, to order a browser answer after a page event. */
function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

const GARMIN = {
  type: 'signin',
  provider: 'garmin',
  loginUrl: 'https://sso.garmin.com/portal/sso/en-US/sign-in',
  successPatterns: ['/app'],
  cookieDomain: 'garmin.com',
};

let chrome;
beforeEach(async () => {
  chrome = fakeChrome();
  globalThis.chrome = chrome;
  // A fresh worker per test: its listener registers on this test's fake.
  await import(`./background.js?test=${Math.random()}`);
});

describe('the service worker', () => {
  it('answers a ping with its version', () => {
    const page = connect(chrome);
    page.send({ type: 'ping' });
    expect(page.answers).toEqual([{ type: 'pong', version: '0.1.0' }]);
  });

  it('closes a port that does not come from Dravr\'s app, answering nothing', () => {
    for (const from of [{ url: 'https://evil.test/' }, { url: 'https://app.dravr.ai.evil.test/' }, { name: 'other' }]) {
      const page = connect(chrome, from);
      page.send({ type: 'ping' });
      expect(page.port.disconnected).toBe(true);
      expect(page.answers).toEqual([]);
    }
  });

  it('refuses a sign-in for any other site, opening nothing', async () => {
    const page = connect(chrome);
    page.send({ ...GARMIN, cookieDomain: 'google.com', loginUrl: 'https://accounts.google.com/' });
    await flush();
    expect(page.answers).toEqual([{ type: 'failed', reason: 'refused' }]);
    expect(chrome.tabs.created).toEqual([]);
  });

  it('hands the provider cookies over once the tab lands signed in, and closes it', async () => {
    chrome.cookies.jar = [
      { name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, httpOnly: true, session: true },
    ];
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    expect(chrome.tabs.created).toEqual([GARMIN.loginUrl]);

    chrome.tabs.onUpdated.fire(7, { url: GARMIN.loginUrl }, { id: 7, url: GARMIN.loginUrl });
    await flush();
    expect(page.answers).toEqual([]);

    chrome.tabs.onUpdated.fire(7, { url: 'https://connect.garmin.com/app' }, { id: 7 });
    await flush();
    expect(chrome.cookies.queried).toEqual(['garmin.com']);
    expect(page.answers).toEqual([
      {
        type: 'signed_in',
        provider: 'garmin',
        cookies: [{ name: 'SESSIONID', value: 'v', domain: 'connect.garmin.com', path: '/', secure: true, http_only: true }],
      },
    ]);
    expect(chrome.tabs.removed).toEqual([7]);
    expect(chrome.tabs.onUpdated.size()).toBe(0);
  });

  it('answers closed when the athlete closes the sign-in tab', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    chrome.tabs.onRemoved.fire(7);
    expect(page.answers).toEqual([{ type: 'failed', reason: 'closed' }]);
    expect(chrome.cookies.queried).toEqual([]);
  });

  it('ignores another tab landing on a signed-in page', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    chrome.tabs.onUpdated.fire(99, { url: 'https://connect.garmin.com/app' }, { id: 99 });
    await flush();
    expect(page.answers).toEqual([]);
    expect(chrome.cookies.queried).toEqual([]);
  });

  it('opens the notice when the extension is installed, and not on an update', () => {
    chrome.runtime.onInstalled.fire({ reason: 'update' });
    expect(chrome.tabs.created).toEqual([]);
    chrome.runtime.onInstalled.fire({ reason: 'install' });
    expect(chrome.tabs.created).toEqual([NOTICE_URL]);
  });

  it('stops watching the tab when the page goes away', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    page.port.onDisconnect.fire();
    expect(chrome.tabs.onUpdated.size()).toBe(0);
    expect(chrome.tabs.onRemoved.size()).toBe(0);
  });
});

describe('the notice before the first sign-in', () => {
  beforeEach(async () => {
    chrome = fakeChrome({ stored: {} });
    globalThis.chrome = chrome;
    await import(`./background.js?test=${Math.random()}`);
  });

  it('opens the notice instead of the provider until the athlete accepts it', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    expect(chrome.tabs.created).toEqual([NOTICE_URL]);
    expect(page.answers).toEqual([]);
  });

  it('starts the sign-in once the athlete accepts, closing the notice', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();

    chrome.storage.accept();
    await flush();

    expect(chrome.tabs.removed).toEqual([7]);
    expect(chrome.tabs.created).toEqual([NOTICE_URL, GARMIN.loginUrl]);
    expect(page.answers).toEqual([]);
  });

  it('answers declined, opening no sign-in, when the athlete closes the notice', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();

    chrome.tabs.onRemoved.fire(7);
    await flush();

    expect(page.answers).toEqual([{ type: 'failed', reason: 'declined' }]);
    expect(chrome.tabs.created).toEqual([NOTICE_URL]);
    expect(chrome.storage.onChanged.size()).toBe(0);
  });

  it('ignores an acceptance of an older notice', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();

    chrome.storage.onChanged.fire({ noticeAccepted: { newValue: { version: 0 } } }, 'local');
    await flush();

    expect(chrome.tabs.created).toEqual([NOTICE_URL]);
  });

  it('stops waiting on the notice when the page goes away', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    page.port.onDisconnect.fire();

    expect(chrome.storage.onChanged.size()).toBe(0);
    expect(chrome.tabs.onRemoved.size()).toBe(0);
    chrome.storage.accept();
    await flush();
    expect(chrome.tabs.created).toEqual([NOTICE_URL]);
  });

  it('answers storage_unreadable, opening nothing, when the acceptance cannot be read', async () => {
    chrome.storage.local.get = async () => {
      throw new Error('storage is unavailable');
    };
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();

    expect(page.answers).toEqual([{ type: 'failed', reason: 'storage_unreadable' }]);
    expect(chrome.tabs.created).toEqual([]);
  });

  it('answers tab_unopened when the notice cannot open, and stops waiting on it', async () => {
    chrome.tabs.create = async () => {
      throw new Error('no window to open a tab in');
    };
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();

    expect(page.answers).toEqual([{ type: 'failed', reason: 'tab_unopened' }]);
    expect(chrome.storage.onChanged.size()).toBe(0);
    expect(chrome.tabs.onRemoved.size()).toBe(0);
  });

  it('tells a page that already left nothing when the acceptance read fails after it', async () => {
    const read = deferred();
    chrome.storage.local.get = () => read.promise;
    const page = connect(chrome);
    page.send(GARMIN);
    page.leave();

    read.reject(new Error('storage is unavailable'));
    await flush();

    expect(page.answers).toEqual([]);
    expect(chrome.tabs.created).toEqual([]);
  });

  it('tells a page that already left nothing when the notice fails to open after it', async () => {
    const opening = deferred();
    chrome.tabs.create = () => opening.promise;
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    page.leave();

    opening.reject(new Error('no window to open a tab in'));
    await flush();

    expect(page.answers).toEqual([]);
  });

  it('tells a page that already left nothing when its notice tab closes', async () => {
    const page = connect(chrome);
    page.send(GARMIN);
    await flush();
    page.port.left = true;

    chrome.tabs.onRemoved.fire(7);
    await flush();

    expect(page.answers).toEqual([]);
    expect(chrome.storage.onChanged.size()).toBe(0);
  });
});
