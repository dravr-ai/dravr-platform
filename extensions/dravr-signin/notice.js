// ABOUTME: The notice page's script: fills its text in the browser's language and records the athlete's acceptance
// ABOUTME: Accepting stores NOTICE_KEY, which the background waits on to start a pending sign-in (carnet#1617)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import { NOTICE_KEY, NOTICE_VERSION, noticeAccepted } from './signin.js';

const api = globalThis.browser ?? globalThis.chrome;

// Each element names its message; the English in notice.html stays when the
// browser has no translation (or the page is opened outside the extension).
for (const element of document.querySelectorAll('[data-i18n]')) {
  const text = api?.i18n?.getMessage(element.dataset.i18n);
  if (text) element.textContent = text;
}
// The language of the messages actually served, which is English when the
// browser's own language has no translation.
document.documentElement.lang = api?.i18n?.getMessage('noticeLang') || 'en';

const showDone = () => {
  document.getElementById('actions').hidden = true;
  document.getElementById('done').hidden = false;
};

if (api?.storage) {
  api.storage.local.get(NOTICE_KEY).then((stored) => {
    if (noticeAccepted(stored)) showDone();
  });
}

document.getElementById('accept').addEventListener('click', () => {
  api.storage.local
    .set({ [NOTICE_KEY]: { version: NOTICE_VERSION, acceptedAt: new Date().toISOString() } })
    .then(showDone);
});

document.getElementById('decline').addEventListener('click', () => window.close());
