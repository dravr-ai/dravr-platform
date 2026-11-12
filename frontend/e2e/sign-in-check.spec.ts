// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Playwright E2E for the login takeover — its page (/sign-in-check#token=…) and the sign-in window — on a mocked socket
// ABOUTME: Frames render at their size, typed and pasted text arrives exact, statuses show, the outcome connects, leaving closes

import { test, expect, type Page, type WebSocketRoute } from '@playwright/test';
import { loginToDashboard, setupDashboardMocks } from './test-helpers';

/** A takeover token shaped as the server mints one: its `tgt` names the provider. */
function takeoverToken(): string {
  const b64url = (json: string) => Buffer.from(json, 'utf8').toString('base64url');
  return `${b64url('{"alg":"HS256","typ":"JWT"}')}.${b64url('{"tgt":"sciotte_garmin","scope":"provider:sciotte:takeover"}')}.signature`;
}

const TOKEN = takeoverToken();
const FRAME = { width: 320, height: 568 };

async function setupBasicMocks(page: Page) {
  await page.route('**/admin/setup/status', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ needs_setup: false, admin_user_exists: true, message: 'Admin user configured' }),
    }),
  );
  await page.route('**/api/auth/session', (route) =>
    route.fulfill({ status: 401, contentType: 'application/json', body: '{}' }),
  );
}

/** A real JPEG frame, drawn by the browser itself so the canvas decodes a true image. */
async function jpegFrame(page: Page): Promise<Buffer> {
  const bytes = await page.evaluate(async ({ width, height }) => {
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('no 2d context');
    context.fillStyle = '#3366cc';
    context.fillRect(0, 0, width, height);
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/jpeg', 0.8));
    if (!blob) throw new Error('no jpeg');
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  }, FRAME);
  return Buffer.from(bytes);
}

/** The mocked takeover socket: what the page sent, whether it closed, and the server's side. */
interface Takeover {
  socket: () => WebSocketRoute;
  token: () => string | null;
  received: () => Array<Record<string, unknown>>;
  closed: () => boolean;
}

async function mockTakeover(page: Page, frame: Buffer): Promise<Takeover> {
  let route: WebSocketRoute | null = null;
  let token: string | null = null;
  const received: Array<Record<string, unknown>> = [];
  let closed = false;
  await page.routeWebSocket('**/ws/providers/sciotte/takeover**', (ws) => {
    route = ws;
    token = new URL(ws.url()).searchParams.get('token');
    ws.onMessage((message) => {
      if (typeof message === 'string') received.push(JSON.parse(message) as Record<string, unknown>);
    });
    ws.onClose(() => {
      closed = true;
    });
    ws.send(JSON.stringify({ type: 'status', state: 'ready', ...FRAME }));
    ws.send(frame);
  });
  return {
    socket: () => {
      if (!route) throw new Error('the page opened no takeover socket');
      return route;
    },
    token: () => token,
    received: () => received,
    closed: () => closed,
  };
}

async function openTakeover(page: Page) {
  await setupBasicMocks(page);
  await page.goto('about:blank');
  const frame = await jpegFrame(page);
  const takeover = await mockTakeover(page, frame);
  await page.goto(`/sign-in-check#token=${encodeURIComponent(TOKEN)}`);
  await expect(page.getByTestId('takeover-status')).toHaveText(
    "Garmin wants to check you're not a robot. Complete the check below, then press Sign In.",
  );
  return takeover;
}

/** The text and key messages the page sent, in order. */
function typed(takeover: Takeover) {
  return takeover.received().filter((m) => m.type === 'text' || m.type === 'key');
}

test.describe('Login takeover page', () => {
  test('opens the socket with the link token, renders frames, and takes the token out of the address bar', async ({ page }) => {
    const takeover = await openTakeover(page);
    expect(takeover.token()).toBe(TOKEN);

    const canvas = page.getByRole('img', { name: 'Garmin sign-in page' });
    await expect(canvas).toBeVisible();
    await expect
      .poll(() => canvas.evaluate((c: HTMLCanvasElement) => [c.width, c.height]))
      .toEqual([FRAME.width, FRAME.height]);
    await expect(page).toHaveURL((url) => url.pathname === '/sign-in-check' && url.hash === '');
  });

  test('sends typed text exactly, repeated letters included, and pasted text whole', async ({ page }) => {
    const takeover = await openTakeover(page);
    const canvas = page.getByRole('img', { name: 'Garmin sign-in page' });
    await expect.poll(() => canvas.evaluate((c: HTMLCanvasElement) => c.width)).toBe(FRAME.width);

    // A mouse press on the page focuses its keyboard.
    await canvas.click({ position: { x: 20, y: 20 } });
    const pointer = takeover.received().filter((m) => m.type === 'pointer');
    expect(pointer.map((m) => m.action)).toEqual(expect.arrayContaining(['down', 'up']));

    await page.keyboard.type('aabbcc');
    await page.keyboard.press('Backspace');
    await page.keyboard.press('Enter');
    await expect.poll(() => typed(takeover).length).toBe(10);
    expect(typed(takeover)).toEqual([
      ...'aabbcc'.split('').map((text) => ({ type: 'text', text })),
      { type: 'key', action: 'down', key: 'Backspace' },
      { type: 'key', action: 'up', key: 'Backspace' },
      { type: 'key', action: 'down', key: 'Enter' },
      { type: 'key', action: 'up', key: 'Enter' },
    ]);

    await page.getByTestId('takeover-keyboard-field').evaluate((field) => {
      const data = new DataTransfer();
      data.setData('text/plain', 'p@ssw0rd!!');
      field.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
    });
    await expect.poll(() => typed(takeover).at(-1)).toEqual({ type: 'text', text: 'p@ssw0rd!!' });
  });

  test('says why a popup did nothing', async ({ page }) => {
    const takeover = await openTakeover(page);
    takeover.socket().send(JSON.stringify({ type: 'popup_refused' }));
    await expect(page.getByTestId('takeover-notice')).toHaveText(
      "That option opens a new window, which doesn't work here. Sign in with your Garmin email and password instead.",
    );
  });

  test('shows the connected state when the takeover connects', async ({ page }) => {
    const takeover = await openTakeover(page);
    takeover.socket().send(
      JSON.stringify({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }),
    );
    await takeover.socket().close();
    await expect(page.getByTestId('sign-in-check-connected')).toContainText('Connected!');
  });

  test('says the sign-in window closed when the socket ends with no outcome', async ({ page }) => {
    const takeover = await openTakeover(page);
    await takeover.socket().close();
    await expect(page.getByTestId('sign-in-check-closed')).toHaveText(
      'The Garmin sign-in window closed. Try connecting again.',
    );
  });

  test('leaving the page closes the socket, which ends the sign-in', async ({ page }) => {
    const takeover = await openTakeover(page);
    expect(takeover.closed()).toBe(false);
    await page.goto('about:blank');
    await expect.poll(() => takeover.closed()).toBe(true);
  });
});

test.describe('Login takeover page on a short phone', () => {
  test.use({ viewport: { width: 390, height: 506 } });

  test('keeps the view and its keyboard button on screen', async ({ page }) => {
    await openTakeover(page);
    await expect(page.getByRole('img', { name: 'Garmin sign-in page' })).toBeInViewport();
    await expect(page.getByRole('button', { name: 'Show keyboard' })).toBeInViewport();
  });
});

test.describe('Login takeover in the sign-in window', () => {
  test('a robot check opens the takeover in the window, and its outcome connects', async ({ page }) => {
    await setupDashboardMocks(page, { role: 'user', skipProvidersRoute: true });
    await page.route('**/api/providers', (route) =>
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          providers: [
            {
              provider: 'sciotte_garmin',
              display_name: 'Garmin',
              requires_oauth: false,
              connected: false,
              capabilities: ['activities'],
              consent_required: false,
            },
          ],
        }),
      }),
    );
    const logins: Array<Record<string, unknown>> = [];
    await page.route('**/api/providers/sciotte/login', async (route) => {
      logins.push(route.request().postDataJSON() as Record<string, unknown>);
      await route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          status: 'takeover_required',
          provider: 'sciotte_garmin',
          takeover_url: `http://localhost/sign-in-check#token=${encodeURIComponent(TOKEN)}`,
        }),
      });
    });
    await page.goto('about:blank');
    const takeover = await mockTakeover(page, await jpegFrame(page));
    await loginToDashboard(page);
    await page.getByRole('button', { name: 'Settings', exact: true }).first().click();
    await page.getByRole('button', { name: 'Data Providers' }).click();
    await page.getByTestId('provider-row-sciotte_garmin').getByRole('button', { name: 'Connect', exact: true }).click();

    const dialog = page.getByRole('dialog');
    await dialog.getByLabel(/email/i).fill('athlete@example.com');
    await dialog.getByLabel('Password', { exact: true }).fill('not-a-real-password');
    await dialog.getByRole('button', { name: 'Log In' }).click();

    await expect(dialog.getByTestId('takeover-status')).toHaveText(
      "Garmin wants to check you're not a robot. Complete the check below, then press Sign In.",
    );
    expect(takeover.token()).toBe(TOKEN);
    expect(logins[0]?.attempt_id).toEqual(expect.stringMatching(/^[0-9a-f-]{36}$/));
    const canvas = dialog.getByRole('img', { name: 'Garmin sign-in page' });
    await expect.poll(() => canvas.evaluate((c: HTMLCanvasElement) => c.width)).toBe(FRAME.width);

    // The page's Escape goes to the page, not to the window around it.
    await canvas.click({ position: { x: 20, y: 20 } });
    await page.keyboard.press('Escape');
    await expect(dialog).toBeVisible();
    await expect
      .poll(() => typed(takeover).filter((m) => m.key === 'Escape').length)
      .toBe(2);

    takeover.socket().send(
      JSON.stringify({ type: 'outcome', status_code: 200, body: { status: 'connected', provider: 'sciotte_garmin' } }),
    );
    await takeover.socket().close();
    await expect(page.getByRole('dialog')).toBeHidden();
  });
});

