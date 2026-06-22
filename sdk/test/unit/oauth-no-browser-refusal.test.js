// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Regression tests for the non-interactive OAuth refusal guard
// ABOUTME: Proves redirectToAuthorization throws (no port bind, no browser, no hang) when disabled

const net = require('net');
const { PierreOAuthClientProvider } = require('../../dist/index.js');

// Count TCP servers the process opens during a call. The bug was that
// redirectToAuthorization started a callback server (binding a port) and then
// awaited a callback that never arrives — orphaning the bridge and leaking the
// port. The fix must refuse BEFORE binding anything, so no new listener appears.
function makeConfig(overrides) {
  return {
    mode: 'oauth',
    pierreServerUrl: 'http://localhost:8081',
    callbackPort: 0,
    oauthClientId: '',
    oauthClientSecret: '',
    ...overrides,
  };
}

const AUTH_URL = new URL(
  'http://localhost:8081/oauth2/login?client_id=test&response_type=code&code_challenge_method=S256',
);

describe('redirectToAuthorization non-interactive refusal', () => {
  const savedEnv = {};
  beforeEach(() => {
    savedEnv.PIERRE_DISABLE_BROWSER = process.env.PIERRE_DISABLE_BROWSER;
    savedEnv.CI = process.env.CI;
    savedEnv.GITHUB_ACTIONS = process.env.GITHUB_ACTIONS;
    delete process.env.PIERRE_DISABLE_BROWSER;
    delete process.env.CI;
    delete process.env.GITHUB_ACTIONS;
  });
  afterEach(() => {
    for (const [k, v] of Object.entries(savedEnv)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }
  });

  test('throws when disableBrowser config is set (--no-browser) without binding a port', async () => {
    const provider = new PierreOAuthClientProvider(
      'http://localhost:8081',
      makeConfig({ disableBrowser: true }),
    );

    // Track new listeners created during the call.
    const before = net.Server.prototype.listen;
    let listenCalls = 0;
    net.Server.prototype.listen = function patched(...args) {
      listenCalls += 1;
      return before.apply(this, args);
    };
    try {
      await expect(
        provider.redirectToAuthorization(AUTH_URL),
      ).rejects.toThrow(/browser opening is disabled/i);
    } finally {
      net.Server.prototype.listen = before;
    }

    expect(listenCalls).toBe(0); // refused BEFORE startCallbackServer
  });

  test('throws when PIERRE_DISABLE_BROWSER=true even if config flag is unset', async () => {
    process.env.PIERRE_DISABLE_BROWSER = 'true';
    const provider = new PierreOAuthClientProvider(
      'http://localhost:8081',
      makeConfig({ disableBrowser: false }),
    );
    await expect(
      provider.redirectToAuthorization(AUTH_URL),
    ).rejects.toThrow(/browser opening is disabled/i);
  });

  test('throws under CI=true (no display)', async () => {
    process.env.CI = 'true';
    const provider = new PierreOAuthClientProvider(
      'http://localhost:8081',
      makeConfig({ disableBrowser: false }),
    );
    await expect(
      provider.redirectToAuthorization(AUTH_URL),
    ).rejects.toThrow(/browser opening is disabled/i);
  });

  test('rejects fast (does not hang) — completes well under the callback-wait timeout', async () => {
    const provider = new PierreOAuthClientProvider(
      'http://localhost:8081',
      makeConfig({ disableBrowser: true }),
    );
    const start = process.hrtime.bigint();
    await expect(
      provider.redirectToAuthorization(AUTH_URL),
    ).rejects.toThrow();
    const elapsedMs = Number(process.hrtime.bigint() - start) / 1e6;
    expect(elapsedMs).toBeLessThan(1000); // immediate refusal, never awaits a callback
  });
});
