// @vitest-environment jsdom
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Runs both hosted sign-in pages' own scripts: a refused code keeps the page on its code step, empty and focused
// ABOUTME: Pins the inline refusal, the code label naming the provider, the right code connecting, and a lapsed sign-in's copy

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { JSDOM, VirtualConsole } from 'jsdom';
import { afterEach, describe, expect, it, vi } from 'vitest';

const ROOT = path.resolve(__dirname, '../../..');
const TEMPLATES = path.join(ROOT, 'crates/pierre-routes-auth/templates');
// The locale catalogue is data, not code under test: it supplies the English
// strings the Rust renderer would hand each page.
const EN: Record<string, unknown> = JSON.parse(
  readFileSync(path.join(ROOT, 'packages/i18n/src/locales/en/translation.json'), 'utf8'),
);

/** The English catalogue's text of a dotted key. */
function text(key: string): string {
  const value = key.split('.').reduce<unknown>((node, part) => (node as Record<string, unknown>)[part], EN);
  if (typeof value !== 'string') throw new Error(`the catalogue has no ${key}`);
  return value;
}

function fill(template: string, value: string): string {
  return template.split('{0}').join(value);
}

const PROVIDER = 'COROS';
const CODE_REJECTED = text('shell.sciotteCodeRejected');
const SIGN_IN_EXPIRED = text('hosted.common.signInExpired');

type Page = 'sciotte_link_login' | 'connect_hosted';

/** The strings each page's renderer hands its script. */
function scriptStrings(page: Page): Record<string, string> {
  const shared = {
    loading: text('hosted.common.loading'),
    verifyingCode: text('hosted.common.verifyingCode'),
    codeRejected: CODE_REJECTED,
    signInExpired: SIGN_IN_EXPIRED,
    linkExpired: text('hosted.error.invalidLink'),
  };
  if (page === 'sciotte_link_login') {
    return {
      ...shared,
      connectingTo: fill(text('hosted.common.connectingTo'), PROVIDER),
      signInRejected: fill(text('hosted.common.signInRejected'), PROVIDER),
      signInUnavailable: fill(text('hosted.common.signInUnavailable'), PROVIDER),
    };
  }
  return {
    ...shared,
    email: text('hosted.common.emailLabel'),
    username: text('hosted.common.usernameLabel'),
    tagConnected: text('hosted.picker.tagConnected'),
    tagAuthorize: text('hosted.picker.tagAuthorize'),
    tagApiKey: text('hosted.picker.tagApiKey'),
    tagUsernamePassword: text('hosted.picker.tagUsernamePassword'),
    tagEmailPassword: text('hosted.picker.tagEmailPassword'),
    accountTitle: text('hosted.common.accountTitle'),
    credentialsNote: text('hosted.common.credentialsNote'),
    connectingTo: text('hosted.common.connectingTo'),
    otpLabel: text('hosted.common.otpLabelNamed'),
    signInRejected: text('hosted.common.signInRejected'),
    signInUnavailable: text('hosted.common.signInUnavailable'),
    stravaFallback: text('hosted.picker.stravaFallback'),
    signInIncomplete: text('hosted.picker.signInIncomplete'),
  };
}

/** The page as its renderer serves it, with English strings and one COROS card. */
function renderPage(page: Page): string {
  const values: Record<string, string> = {
    STRINGS_JSON: JSON.stringify(scriptStrings(page)),
    FLOW_EXPIRED_REASON: 'login_flow_expired',
    CODE_REJECTED_REASON: 'code_rejected',
    CONSENT_REQUIRED: 'false',
    CONSENT_HIDDEN: ' hidden',
    PROVIDERS_JSON: JSON.stringify([
      {
        provider: 'sciotte_coros',
        display_name: PROVIDER,
        description: '',
        connected: false,
        kind: 'sciotte',
        target: 'coros',
        consent_required: false,
        login_identifier: 'email',
      },
    ]),
    LINK_TOKEN: 'link-token',
    TARGET: 'coros',
    CHANNEL: 'telegram',
    ID_TYPE: 'email',
    ID_AUTOCOMPLETE: 'email',
    ID_LABEL: text('hosted.common.emailLabel'),
    OTP_LABEL: fill(text('hosted.common.otpLabelNamed'), PROVIDER),
  };
  // The template is the artefact under test: its inline script only runs in a
  // browser, so the test serves it as the renderer would and drives that script.
  return readFileSync(path.join(TEMPLATES, `${page}.html`), 'utf8')
    .replace(/\{\{t:([A-Za-z.]+)\}\}/g, (_, key: string) => text(key))
    .replace(/\{\{([A-Z_]+)\}\}/g, (_, name: string) => values[name] ?? '');
}

/** One scripted answer of the sign-in API, by path. */
type Answer = { status: number; body: unknown };

let dom: JSDOM | null = null;

afterEach(() => {
  dom?.window.close();
  dom = null;
});

/** Load `page` with its script running against `answers`, popped in order per path. */
function open(page: Page, answers: Record<string, Answer[]>) {
  const calls: Array<{ path: string; body: unknown }> = [];
  const virtualConsole = new VirtualConsole();
  dom = new JSDOM(renderPage(page), {
    runScripts: 'dangerously',
    url: `https://hosted.test/providers/${page}?token=link-token`,
    virtualConsole,
    beforeParse(window) {
      Object.assign(window, {
        fetch: async (url: string, init: { body: string }) => {
          const queue = answers[url];
          if (!queue || queue.length === 0) throw new Error(`no answer scripted for ${url}`);
          calls.push({ path: url, body: JSON.parse(init.body) });
          const answer = queue.shift() as Answer;
          return {
            ok: answer.status >= 200 && answer.status < 300,
            status: answer.status,
            text: async () => JSON.stringify(answer.body),
          };
        },
      });
    },
  });
  const document = dom.window.document;
  const byId = (id: string) => document.getElementById(id) as HTMLElement;
  const submit = (formId: string) =>
    byId(formId).dispatchEvent(new dom!.window.Event('submit', { bubbles: true, cancelable: true }));
  const visible = (id: string) => !byId(id).hidden;
  return { document, byId, submit, visible, calls };
}

const LOGIN = '/api/providers/sciotte/login';
const SUBMIT = '/api/providers/sciotte/submit-otp';

async function reachCodeStep(page: Page, answers: Record<string, Answer[]>) {
  const view = open(page, { [LOGIN]: [{ status: 200, body: { status: 'otp_required' } }], ...answers });
  if (page === 'connect_hosted') {
    (view.document.querySelector('.provider-card') as HTMLButtonElement).click();
  }
  expect(view.visible('phase-credentials')).toBe(true);
  (view.byId('email') as HTMLInputElement).value = 'athlete@example.com';
  (view.byId('password') as HTMLInputElement).value = 'not-a-real-password';
  view.submit('login-form');
  await vi.waitFor(() => expect(view.visible('phase-otp')).toBe(true));
  return view;
}

async function enterCode(view: ReturnType<typeof open>, code: string, until: () => void) {
  (view.byId('otp-code') as HTMLInputElement).value = code;
  view.submit('otp-form');
  await vi.waitFor(until);
}

describe.each<Page>(['sciotte_link_login', 'connect_hosted'])('%s — the code step', (page) => {
  it('stays on the code step when a code is refused, then connects on the right one', async () => {
    const view = await reachCodeStep(page, {
      [SUBMIT]: [
        { status: 200, body: { status: 'otp_required', reason: 'code_rejected' } },
        { status: 200, body: { status: 'connected', provider: 'sciotte_coros' } },
      ],
    });
    expect(view.visible('otp-error')).toBe(false);
    expect(view.byId('otp-label').textContent).toBe(fill(text('hosted.common.otpLabelNamed'), PROVIDER));

    await enterCode(view, '000000', () => expect(view.visible('otp-error')).toBe(true));
    expect(view.visible('phase-otp')).toBe(true);
    expect(view.byId('otp-error').textContent).toBe(CODE_REJECTED);
    const input = view.byId('otp-code') as HTMLInputElement;
    expect(input.value).toBe('');
    expect(view.document.activeElement).toBe(input);
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(input.getAttribute('maxlength')).toBe('6');

    await enterCode(view, '246810', () => expect(view.visible('phase-success')).toBe(true));
    expect(view.calls.filter((c) => c.path === SUBMIT).map((c) => c.body)).toEqual([
      { code: '000000' },
      { code: '246810' },
    ]);
  });

  it('asks to start again when the sign-in lapsed', async () => {
    const view = await reachCodeStep(page, {
      [SUBMIT]: [
        {
          status: 400,
          body: { message: 'server prose', details: { reason: 'login_flow_expired' } },
        },
      ],
    });
    await enterCode(view, '246810', () => expect(view.visible('phase-error')).toBe(true));
    expect(view.byId('error-message').textContent).toBe(SIGN_IN_EXPIRED);
  });
});
