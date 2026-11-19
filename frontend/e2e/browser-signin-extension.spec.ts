// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: E2E for the Dravr sign-in extension (carnet#1617): the real extension, loaded in Chromium, connects Garmin from the athlete's browser
// ABOUTME: The app is served at https://app.dravr.ai (routed to the Vite server) so bridge.js injects; Garmin is a local server, the API mocked

import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer, type Server } from 'node:https';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, expect, test, type BrowserContext, type Page, type Route } from '@playwright/test';
import { APP_SHELL_TIMEOUT_MS, setupDashboardMocks, signInThroughHostedPage } from './test-helpers';

/** The unpacked extension, exactly as a developer loads it. */
const EXTENSION_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../../extensions/dravr-signin');
/** The origin bridge.js runs on: the app is served there, from the Vite server the config starts. */
const APP_ORIGIN = 'https://app.dravr.ai';
const DEV_SERVER = `http://localhost:${process.env.E2E_PORT ?? '5174'}`;

const GARMIN_SIGN_IN = 'https://sso.garmin.com/portal/sso/en-US/sign-in';
const GARMIN_SIGNED_IN = 'https://connect.garmin.com/app/home';
const GARMIN_ENTRY = {
  provider: 'garmin',
  login_url: GARMIN_SIGN_IN,
  success_patterns: ['connect.garmin.com/app'],
  cookie_domain: 'garmin.com',
};

const json = (route: Route, body: unknown, status = 200) =>
  route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });

/** The Garmin card with its history still to copy: Add history opens the sign-in window. */
const garminCard = {
  provider: 'sciotte_garmin',
  display_name: 'Garmin',
  description: 'Activities and health metrics from Garmin devices',
  requires_oauth: false,
  connected: true,
  connected_backend: 'garmin',
  connection_type: 'oauth',
  needs_reauth: false,
  capabilities: ['activities', 'sleep', 'health'],
  consent_required: false,
  recommended_backend: 'oauth',
  history: { state: 'needed', sign_in_target: 'sciotte_garmin', archive_source: 'garmin_archive' },
};

/**
 * Garmin, played by a local HTTPS server: its sign-in page sends the athlete
 * on to the signed-in page, which sets the session cookie, as Garmin does.
 * The tab the extension opens is a page Playwright's routes do not reach, so
 * Chromium resolves every garmin.com host here instead: no test can ever
 * talk to the real Garmin.
 */
async function startGarmin(): Promise<{ server: Server; port: number; stop: () => void }> {
  const dir = mkdtempSync(path.join(tmpdir(), 'dravr-e2e-garmin-'));
  const key = path.join(dir, 'key.pem');
  const cert = path.join(dir, 'cert.pem');
  execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', key, '-out', cert, '-days', '1', '-subj', '/CN=garmin.com'], {
    stdio: 'ignore',
  });
  const server = createServer({ key: readFileSync(key), cert: readFileSync(cert) }, (request, response) => {
    const url = new URL(request.url ?? '/', `https://${request.headers.host}`);
    if (url.host === 'sso.garmin.com' && url.pathname === new URL(GARMIN_SIGN_IN).pathname) {
      response.writeHead(200, { 'content-type': 'text/html' });
      response.end(`<!doctype html><title>Garmin sign-in</title><script>location.replace(${JSON.stringify(GARMIN_SIGNED_IN)})</script>`);
    } else if (url.host === 'connect.garmin.com' && url.pathname.startsWith('/app')) {
      response.writeHead(200, {
        'content-type': 'text/html',
        'set-cookie': 'SESSIONID=e2e-garmin-session; Domain=connect.garmin.com; Path=/; Secure; HttpOnly; SameSite=Lax',
      });
      response.end('<!doctype html><title>Garmin Connect</title><p>Signed in</p>');
    } else {
      response.writeHead(404);
      response.end();
    }
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const { port } = server.address() as AddressInfo;
  return { server, port, stop: () => { server.close(); rmSync(dir, { recursive: true, force: true }); } };
}

/**
 * A Chromium with the extension installed, the app at app.dravr.ai and every
 * garmin.com host resolved to `garminPort`. The app's API is mocked per test;
 * an /api call no test mocked answers 404 here and never leaves the machine.
 */
async function launchWithExtension(garminPort: number): Promise<BrowserContext> {
  const context = await chromium.launchPersistentContext('', {
    channel: 'chromium',
    headless: true,
    baseURL: APP_ORIGIN,
    args: [
      `--disable-extensions-except=${EXTENSION_DIR}`,
      `--load-extension=${EXTENSION_DIR}`,
      `--host-resolver-rules=MAP *.garmin.com 127.0.0.1:${garminPort}`,
      // The local Garmin's certificate is self-signed.
      '--ignore-certificate-errors',
    ],
  });
  await context.route(`${APP_ORIGIN}/**`, async (route) => {
    const url = new URL(route.request().url());
    if (url.pathname.startsWith('/api/')) {
      await route.fulfill({ status: 404, contentType: 'application/json', body: '{}' });
      return;
    }
    try {
      const response = await route.fetch({ url: `${DEV_SERVER}${url.pathname}${url.search}` });
      await route.fulfill({ response });
    } catch {
      // The test closed the browser while Vite was answering.
    }
  });
  await context.addInitScript(() => localStorage.setItem('pierre_app_language', 'en'));
  return context;
}

/**
 * The hosted sign-in, answered as `mockHostedSignIn` answers it but returning
 * by a navigation instead of a 302: Chromium follows a redirect a route
 * fulfilled without routing its target, so a 302 to app.dravr.ai would load
 * the real site instead of the Vite server.
 */
async function signIn(page: Page) {
  await page.route('**/oauth2/authorize**', (route) => {
    const request = new URL(route.request().url());
    const callback = new URL(request.searchParams.get('redirect_uri') ?? `${APP_ORIGIN}/auth/callback`);
    callback.searchParams.set('code', 'e2e-code');
    callback.searchParams.set('state', request.searchParams.get('state') ?? '');
    return route.fulfill({
      contentType: 'text/html',
      body: `<!doctype html><script>location.replace(${JSON.stringify(callback.toString())})</script>`,
    });
  });
  await page.goto('/');
  await signInThroughHostedPage(page, { mockHostedPage: false });
  await page.waitForSelector('main', { timeout: APP_SHELL_TIMEOUT_MS });
}

/** The page the extension opened, once it has one at `url`. */
async function tabAt(context: BrowserContext, url: RegExp): Promise<Page> {
  const open = context.pages().find((page) => url.test(page.url()));
  if (open) return open;
  return context.waitForEvent('page', { predicate: (page) => url.test(page.url()), timeout: 15000 });
}

test.describe('Sign in to Garmin through the Dravr extension (carnet#1617)', () => {
  let garmin: Awaited<ReturnType<typeof startGarmin>>;
  let context: BrowserContext;
  let page: Page;
  let handedOver: unknown;

  test.beforeAll(async () => {
    garmin = await startGarmin();
  });

  test.afterAll(() => {
    garmin?.stop();
  });

  test.beforeEach(async () => {
    handedOver = undefined;
    context = await launchWithExtension(garmin.port);
    // A new install opens the extension's notice; the athlete closes it unread.
    const installNotice = await tabAt(context, /notice\.html$/);
    await installNotice.close();

    page = await context.newPage();
    await setupDashboardMocks(page, { role: 'user', email: 'alice@acme.com', displayName: 'Alice Test' });
    await page.route('**/api/providers', (route) => json(route, { providers: [garminCard] }));
    await page.route('**/api/imports', (route) =>
      json(route, { enabled: true, sources: ['garmin_archive'], imports: [] }),
    );
    await page.route('**/api/imports/garmin_archive/current', (route) => json(route, { import: null }));
    await page.route('**/api/providers/browser-signin', (route) => json(route, { providers: [GARMIN_ENTRY] }));
    await page.route('**/api/providers/browser-signin/garmin', (route) => {
      handedOver = route.request().postDataJSON();
      return json(route, { status: 'connected', provider: 'sciotte_garmin' });
    });
  });

  test.afterEach(async () => {
    await context?.close();
  });

  async function openGarminSignIn() {
    await signIn(page);
    await page.goto('/#settings/connections');
    const row = page.getByTestId('provider-row-sciotte_garmin');
    await expect(row).toBeVisible({ timeout: APP_SHELL_TIMEOUT_MS });
    await row.getByRole('button', { name: 'Add history' }).click();
    const login = page.getByRole('dialog', { name: /Garmin/ });
    await expect(login).toBeVisible({ timeout: 10000 });
    return login;
  }

  test('the athlete accepts the notice, signs in to Garmin in a tab, and Dravr receives that session', async () => {
    const login = await openGarminSignIn();

    // The extension answered the page: the window offers the browser sign-in.
    await login.getByRole('button', { name: 'Continue with Garmin' }).click();

    // First use: the notice comes first, and accepting it starts the sign-in.
    const notice = await tabAt(context, /notice\.html$/);
    await expect(notice.getByRole('heading', { name: 'Before you use Dravr sign-in' })).toBeVisible();
    await notice.getByRole('button', { name: 'I agree' }).click();

    await expect.poll(() => handedOver, { timeout: 15000 }).toBeTruthy();
    // Exactly the cookie Garmin set, as Chromium keeps a Domain cookie, and nothing else.
    expect(handedOver).toMatchObject({
      cookies: [
        { name: 'SESSIONID', value: 'e2e-garmin-session', domain: '.connect.garmin.com', path: '/', secure: true, http_only: true },
      ],
    });
    expect((handedOver as { cookies: unknown[] }).cookies).toHaveLength(1);
    // The extension closed both of the tabs it opened: the notice and Garmin's.
    await expect
      .poll(() => context.pages().filter((open) => /notice\.html$|garmin\.com/.test(open.url())).map((open) => open.url()))
      .toEqual([]);
  });

  test('closing the notice returns to the Garmin credentials, handing nothing over', async () => {
    const login = await openGarminSignIn();

    await login.getByRole('button', { name: 'Continue with Garmin' }).click();
    const notice = await tabAt(context, /notice\.html$/);
    // "Not now" closes its own page, which can end before Playwright's click
    // settles: the page closing is the click's expected effect, anything else
    // fails the test.
    const closed = notice.waitForEvent('close');
    await notice
      .getByRole('button', { name: 'Not now' })
      .click()
      .catch((error: unknown) => {
        if (!notice.isClosed()) throw error;
      });
    await closed;

    await expect(login.getByLabel(/email/i)).toBeVisible({ timeout: 10000 });
    expect(handedOver).toBeUndefined();
  });
});
