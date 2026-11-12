// @vitest-environment jsdom
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Runs both hosted sign-in pages' own scripts: a refused code keeps the page on its code step, empty and focused
// ABOUTME: Pins the inline refusal, a lapsed or challenged sign-in, the pagehide cancel, a Garmin grant's history sign-in, the OAuth fallback

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { JSDOM, VirtualConsole } from 'jsdom';
// jsdom has no navigation: its `Location` implementation is reached to record
// where the page's script sends the browser.
import { implForWrapper } from 'jsdom/lib/jsdom/living/generated/utils.js';
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
const SIGN_IN_CHALLENGED = fill(text('hosted.common.signInChallenged'), PROVIDER);

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
      signInChallenged: SIGN_IN_CHALLENGED,
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
    signInChallenged: text('hosted.common.signInChallenged'),
    stravaFallback: text('hosted.picker.stravaFallback'),
    signInIncomplete: text('hosted.picker.signInIncomplete'),
    tagImportHistory: text('hosted.picker.tagImportHistory'),
    importExportLink: text('hosted.picker.importExportLink'),
    historySignInNote: text('hosted.picker.historySignInNote'),
    historyCopying: text('hosted.picker.historyCopying'),
    oauthRetry: text('hosted.picker.oauthRetry'),
  };
}

/** The one COROS card the code-step tests pick. */
const COROS_CARDS = [
  {
    provider: 'sciotte_coros',
    display_name: PROVIDER,
    description: '',
    connected: false,
    kind: 'credentials',
    target: 'coros',
    consent_required: false,
    login_identifier: 'email',
  },
];

/** The page as its renderer serves it, with English strings and `cards` (one COROS card by default). */
function renderPage(page: Page, cards: unknown[] = COROS_CARDS): string {
  const values: Record<string, string> = {
    STRINGS_JSON: JSON.stringify(scriptStrings(page)),
    FLOW_EXPIRED_REASON: 'login_flow_expired',
    LOGIN_CHALLENGE_REASON: 'login_challenge',
    CODE_REJECTED_REASON: 'code_rejected',
    SIGN_IN_UNAVAILABLE_REASON: 'sign_in_unavailable',
    CONSENT_REQUIRED: 'false',
    CONSENT_HIDDEN: ' hidden',
    PROVIDERS_JSON: JSON.stringify(cards),
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

/**
 * Load `page` with its script running against `answers`, popped in order per
 * path; `cards` and `search` (extra query, such as an OAuth return's) default
 * to the code-step tests' one COROS card and none.
 */
function open(page: Page, answers: Record<string, Answer[]>, cards?: unknown[], search = '') {
  const calls: Array<{ path: string; body: unknown; keepalive?: boolean; authorization?: string }> = [];
  const navigations: string[] = [];
  const virtualConsole = new VirtualConsole();
  dom = new JSDOM(renderPage(page, cards), {
    runScripts: 'dangerously',
    url: `https://hosted.test/providers/${page}?token=link-token${search}`,
    virtualConsole,
    beforeParse(window) {
      (implForWrapper(window.location) as { assign: (url: string) => void }).assign = (url) => {
        navigations.push(url);
      };
      Object.assign(window, {
        fetch: async (
          url: string,
          init: { body: string; keepalive?: boolean; headers?: Record<string, string> },
        ) => {
          const queue = answers[url];
          if (!queue || queue.length === 0) throw new Error(`no answer scripted for ${url}`);
          calls.push({
            path: url,
            body: JSON.parse(init.body),
            keepalive: init.keepalive,
            authorization: init.headers?.Authorization,
          });
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
  const hide = () => dom!.window.dispatchEvent(new dom!.window.Event('pagehide'));
  return { document, byId, submit, visible, calls, hide, navigations };
}

const LOGIN = '/api/providers/sciotte/login';
const SUBMIT = '/api/providers/sciotte/submit-otp';
const CANCEL = '/api/providers/sciotte/cancel';

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
    const attemptId = (view.calls[0].body as { attempt_id: string }).attempt_id;
    expect(attemptId).toMatch(/^[0-9a-f-]{32,36}$/);
    expect(view.calls.filter((c) => c.path === SUBMIT).map((c) => c.body)).toEqual([
      { code: '000000', attempt_id: attemptId },
      { code: '246810', attempt_id: attemptId },
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

  it('says the provider asked for a check when the sign-in was stopped at a challenge', async () => {
    const view = open(page, {
      [LOGIN]: [
        {
          status: 400,
          body: { message: 'server prose', details: { reason: 'login_challenge' } },
        },
      ],
    });
    if (page === 'connect_hosted') {
      (view.document.querySelector('.provider-card') as HTMLButtonElement).click();
    }
    (view.byId('email') as HTMLInputElement).value = 'athlete@example.com';
    (view.byId('password') as HTMLInputElement).value = 'not-a-real-password';
    view.submit('login-form');
    await vi.waitFor(() => expect(view.visible('phase-error')).toBe(true));
    expect(view.byId('error-message').textContent).toBe(SIGN_IN_CHALLENGED);
  });
});

describe.each<Page>(['sciotte_link_login', 'connect_hosted'])('%s — leaving the page', (page) => {
  /** A cancel the server answers; a fresh queue for each test. */
  const cancelled = () => ({ [CANCEL]: [{ status: 200, body: { cancelled: true } }] });

  it('cancels the attempt its steps named, with a request that outlives the page', async () => {
    const view = await reachCodeStep(page, cancelled());
    const attemptId = (view.calls[0].body as { attempt_id: string }).attempt_id;

    view.hide();

    await vi.waitFor(() => expect(view.calls.some((c) => c.path === CANCEL)).toBe(true));
    const cancel = view.calls.find((c) => c.path === CANCEL)!;
    expect(cancel.body).toEqual({ attempt_id: attemptId });
    expect(cancel.keepalive).toBe(true);
    expect(cancel.authorization).toBe('Bearer link-token');
  });

  it('starts a fresh attempt once the old one is cancelled, and cancels it once', async () => {
    const view = await reachCodeStep(page, {
      ...cancelled(),
      [SUBMIT]: [{ status: 200, body: { status: 'otp_required' } }],
    });
    const first = (view.calls[0].body as { attempt_id: string }).attempt_id;
    view.hide();
    view.hide();

    await enterCode(view, '246810', () =>
      expect(view.calls.some((c) => c.path === SUBMIT)).toBe(true),
    );
    const next = (view.calls.find((c) => c.path === SUBMIT)!.body as { attempt_id: string }).attempt_id;
    expect(next).not.toBe(first);
    expect(view.calls.filter((c) => c.path === CANCEL)).toHaveLength(1);
  });

  it('sends no cancel when nothing is running', async () => {
    const view = open(page, {});
    view.hide();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(view.calls).toEqual([]);
  });
});

describe('connect_hosted — a Garmin grant whose history is needed (carnet#1472)', () => {
  /** The Garmin card as the picker serves it after a grant, its history in `state`. */
  const garminGrant = (state: 'needed', historySignIn = true) => [
    {
      provider: 'garmin',
      display_name: 'Garmin',
      description: '',
      connected: true,
      history_state: state,
      history_sign_in: historySignIn,
      archive_url: '/providers/connect/import?token=link-token&source=garmin_archive',
      migrate: false,
      kind: 'oauth',
      target: 'garmin',
      consent_required: false,
      login_identifier: 'email_or_username',
    },
  ];
  const historyNotice = `${fill(text('hosted.picker.historySignInNote'), 'Garmin')} ${fill(text('hosted.common.credentialsNote'), 'Garmin')}`;

  it('goes straight into the Garmin sign-in, in history mode, when the grant comes back', async () => {
    const view = open(
      'connect_hosted',
      { [LOGIN]: [{ status: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }] },
      garminGrant('needed'),
      '&provider=garmin&success=true',
    );

    expect(view.visible('phase-credentials')).toBe(true);
    expect(view.visible('phase-picker')).toBe(false);
    expect(view.visible('phase-success')).toBe(false);
    expect(view.byId('cred-notice').textContent).toBe(historyNotice);

    (view.byId('email') as HTMLInputElement).value = 'athlete@example.com';
    (view.byId('password') as HTMLInputElement).value = 'not-a-real-password';
    view.submit('login-form');
    await vi.waitFor(() => expect(view.visible('phase-success')).toBe(true));
    expect(view.calls[0].path).toBe(LOGIN);
    expect(view.byId('success-message').textContent).toBe(text('hosted.picker.historyCopying'));
  });

  it('opens the same sign-in in one tap on a card whose history is needed', () => {
    const view = open('connect_hosted', {}, garminGrant('needed'));
    const card = view.document.querySelector('.provider-card') as HTMLButtonElement;
    expect(card.disabled).toBe(false);

    card.click();

    expect(view.visible('phase-credentials')).toBe(true);
    expect(view.byId('cred-notice').textContent).toBe(historyNotice);
  });

  it('opens no sign-in on the return for a card the server names none for', () => {
    const view = open('connect_hosted', {}, garminGrant('needed', false), '&provider=garmin&success=true');
    expect(view.visible('phase-credentials')).toBe(false);
    expect(view.calls).toEqual([]);
  });

  it('leaves a card the server names no sign-in for inert to a tap', () => {
    const view = open('connect_hosted', {}, garminGrant('needed', false));
    const card = view.document.querySelector('.provider-card') as HTMLButtonElement;
    expect(card.disabled).toBe(true);
    card.click();
    expect(view.visible('phase-credentials')).toBe(false);
    expect(view.calls).toEqual([]);
  });
});

describe('connect_hosted — a sign-in-first card falls back to its OAuth (carnet#1477)', () => {
  /** The Strava card as the picker serves it while its sign-in goes first; `fallback` as the server sets it. */
  const stravaSignInFirst = (fallback: boolean) => [
    {
      provider: 'sciotte',
      display_name: 'Strava',
      description: '',
      connected: false,
      history_state: 'not_applicable',
      history_sign_in: false,
      migrate: false,
      kind: 'credentials',
      target: 'strava',
      consent_required: false,
      login_identifier: 'email',
      oauth_fallback: fallback,
    },
  ];
  const OAUTH_INIT = '/api/providers/connect/oauth-init/strava?token=link-token';

  async function signIn(cards: unknown[], answer: Answer) {
    const view = open('connect_hosted', { [LOGIN]: [answer] }, cards);
    (view.document.querySelector('.provider-card') as HTMLButtonElement).click();
    expect(view.visible('phase-credentials')).toBe(true);
    (view.byId('email') as HTMLInputElement).value = 'athlete@example.com';
    (view.byId('password') as HTMLInputElement).value = 'not-a-real-password';
    view.submit('login-form');
    await vi.waitFor(() => expect(view.calls).toHaveLength(1));
    return view;
  }

  it.each(['sign_in_unavailable', 'login_challenge'])('starts the Strava OAuth when the sign-in answers %s', async (reason) => {
    const view = await signIn(stravaSignInFirst(true), {
      status: 400,
      body: { message: 'server prose', details: { reason } },
    });

    await vi.waitFor(() => expect(view.navigations).toEqual([OAUTH_INIT]));
    expect(view.visible('phase-error')).toBe(false);
  });

  it('keeps wrong credentials on the page, for another try', async () => {
    const view = await signIn(stravaSignInFirst(true), { status: 200, body: { status: 'failed', error: 'bad' } });

    await vi.waitFor(() => expect(view.visible('phase-error')).toBe(true));
    expect(view.byId('error-message').textContent).toBe(fill(text('hosted.common.signInRejected'), 'Strava'));
    expect(view.navigations).toEqual([]);
  });

  it('shows the failure on a card the server offers no fallback for', async () => {
    const view = await signIn(stravaSignInFirst(false), {
      status: 400,
      body: { message: 'server prose', details: { reason: 'sign_in_unavailable' } },
    });

    await vi.waitFor(() => expect(view.visible('phase-error')).toBe(true));
    expect(view.byId('error-message').textContent).toBe(fill(text('hosted.common.signInUnavailable'), 'Strava'));
    expect(view.navigations).toEqual([]);
  });

  it('never falls back from the sign-in a refused Strava grant opened', async () => {
    const view = open(
      'connect_hosted',
      {
        [LOGIN]: [{ status: 400, body: { message: 'server prose', details: { reason: 'sign_in_unavailable' } } }],
      },
      stravaSignInFirst(true),
      '&provider=strava&success=false',
    );
    expect(view.visible('phase-credentials')).toBe(true);
    (view.byId('email') as HTMLInputElement).value = 'athlete@example.com';
    (view.byId('password') as HTMLInputElement).value = 'not-a-real-password';
    view.submit('login-form');

    await vi.waitFor(() => expect(view.visible('phase-error')).toBe(true));
    expect(view.navigations).toEqual([]);
  });
});
