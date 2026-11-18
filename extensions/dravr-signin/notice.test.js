// ABOUTME: Tests for notice.js, the notice page's script: its text in the browser's language and the acceptance it records
// ABOUTME: Runs the module against a fake document and browser under `bun test extensions/dravr-signin` (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { beforeEach, describe, expect, it } from 'bun:test';
import { NOTICE_VERSION, noticeAccepted } from './signin.js';

/** One element of the fake page: its message name, text, visibility and click listeners. */
function element(i18n, textContent = '') {
  const clicks = [];
  return {
    dataset: i18n ? { i18n } : {},
    textContent,
    hidden: false,
    addEventListener: (type, listener) => {
      if (type === 'click') clicks.push(listener);
    },
    click: () => clicks.forEach((listener) => listener()),
  };
}

/**
 * The page notice.html renders, reduced to what notice.js touches: the texts
 * named by `data-i18n` (English as written in notice.html), the two buttons,
 * the actions row and the done line (hidden until accepted).
 */
function fakeDocument() {
  const texts = [element('noticeTitle', 'Before you use Dravr sign-in'), element('noticeReads', 'Only the cookies…')];
  const byId = {
    actions: element(null),
    done: { ...element('noticeDone', 'All set.'), hidden: true },
    accept: element('noticeAccept', 'I agree'),
    decline: element('noticeDecline', 'Not now'),
  };
  return {
    texts,
    byId,
    documentElement: { lang: 'en' },
    querySelectorAll: (selector) => (selector === '[data-i18n]' ? [...texts, byId.done, byId.accept, byId.decline] : []),
    getElementById: (id) => byId[id] ?? null,
  };
}

/** The browser the page runs in: its messages, UI language and storage.local. */
function fakeBrowser({ messages = {}, stored = {} } = {}) {
  const browser = {
    written: [],
    i18n: {
      getMessage: (name) => messages[name] ?? '',
    },
    storage: {
      local: {
        get: async (key) => (key in stored ? { [key]: stored[key] } : {}),
        set: async (items) => {
          browser.written.push(items);
        },
      },
    },
  };
  return browser;
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

let page;
let closed;

/** Load a fresh copy of notice.js against this test's page and browser. */
async function openNotice(browser) {
  // Another suite may leave Firefox's namespace behind; notice.js would prefer it.
  globalThis.browser = undefined;
  globalThis.chrome = browser;
  await import(`./notice.js?test=${Math.random()}`);
  await flush();
}

beforeEach(() => {
  page = fakeDocument();
  closed = 0;
  globalThis.document = page;
  globalThis.window = {
    close: () => {
      closed += 1;
    },
  };
});

describe('the notice page', () => {
  it('shows each text in the browser\'s language, keeping the English where no translation exists', async () => {
    await openNotice(fakeBrowser({ messages: { noticeLang: 'fr', noticeTitle: 'Avant d’utiliser la connexion Dravr' } }));

    expect(page.texts.map((text) => text.textContent)).toEqual([
      'Avant d’utiliser la connexion Dravr',
      'Only the cookies…',
    ]);
    expect(page.documentElement.lang).toBe('fr');
  });

  it('tags the page English when no message is served', async () => {
    page.documentElement.lang = 'ja';
    await openNotice(fakeBrowser());

    expect(page.documentElement.lang).toBe('en');
  });

  it('records an acceptance the background starts a sign-in on, then says it is done', async () => {
    const browser = fakeBrowser();
    await openNotice(browser);
    expect(page.byId.done.hidden).toBe(true);

    page.byId.accept.click();
    await flush();

    expect(browser.written).toHaveLength(1);
    const [written] = browser.written;
    expect(written.noticeAccepted.version).toBe(NOTICE_VERSION);
    expect(Number.isNaN(Date.parse(written.noticeAccepted.acceptedAt))).toBe(false);
    expect(noticeAccepted(written)).toBe(true);
    expect(page.byId.actions.hidden).toBe(true);
    expect(page.byId.done.hidden).toBe(false);
    expect(closed).toBe(0);
  });

  it('opens as done when this notice was already accepted', async () => {
    await openNotice(fakeBrowser({ stored: { noticeAccepted: { version: NOTICE_VERSION } } }));

    expect(page.byId.actions.hidden).toBe(true);
    expect(page.byId.done.hidden).toBe(false);
  });

  it('asks again when only an older notice was accepted', async () => {
    await openNotice(fakeBrowser({ stored: { noticeAccepted: { version: NOTICE_VERSION - 1 } } }));

    expect(page.byId.actions.hidden).toBe(false);
    expect(page.byId.done.hidden).toBe(true);
  });

  it('closes on Not now, recording nothing', async () => {
    const browser = fakeBrowser();
    await openNotice(browser);

    page.byId.decline.click();
    await flush();

    expect(closed).toBe(1);
    expect(browser.written).toEqual([]);
  });
});
