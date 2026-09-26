// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Security tests for the local OAuth callback listener (provider token POSTs, port binding)
// ABOUTME: Proves the listener takes no provider tokens and the published port is the one it holds

const net = require('net');
const {
  closeServer,
  freePort,
  httpRequest,
  makeProvider,
  occupyPort,
  startProvider,
  stopProvider,
} = require('./oauth-callback-harness.js');

describe('the callback listener takes no provider tokens', () => {
  test('a provider-token POST is not a route: nothing is stored and no flow is touched', async () => {
    const { provider, port } = await startProvider();
    try {
      const res = await httpRequest(port, {
        method: 'POST',
        path: '/oauth/provider-callback/strava',
        json: { access_token: 'planted-access-token', refresh_token: 'planted-refresh-token' },
      });

      // Dravr completes provider flows on its side and the bridge polls it for the
      // outcome, so the listener answers the Dravr sign-in redirect and nothing else.
      expect(res.status).toBe(404);
      expect(res.body).toBe('Not Found');
      expect(provider.allStoredTokens).toEqual({});
      expect(provider.authorizationPending).toBeUndefined();
    } finally {
      stopProvider(provider);
    }
  });
});

describe('callback port binding', () => {
  const savedEnv = {};

  beforeEach(() => {
    savedEnv.PIERRE_DISABLE_BROWSER = process.env.PIERRE_DISABLE_BROWSER;
    savedEnv.CI = process.env.CI;
    savedEnv.GITHUB_ACTIONS = process.env.GITHUB_ACTIONS;
  });

  afterEach(() => {
    for (const [key, value] of Object.entries(savedEnv)) {
      if (value === undefined) {
        delete process.env[key];
      } else {
        process.env[key] = value;
      }
    }
  });

  test('the published redirect URI names the port the listener actually holds', async () => {
    const { provider, port, boundPort } = await startProvider();
    try {
      expect(boundPort).toBe(port);
      expect(provider.callbackServer.address().port).toBe(port);
      expect(provider.redirectUrl).toBe(`http://localhost:${port}/oauth/callback`);
    } finally {
      stopProvider(provider);
    }
  });

  test('an occupied callback port fails hard and never moves to an unpublished port', async () => {
    const port = await freePort();
    const squatter = await occupyPort(port);

    const attemptedPorts = [];
    const originalListen = net.Server.prototype.listen;
    net.Server.prototype.listen = function patched(...args) {
      if (typeof args[0] === 'number') {
        attemptedPorts.push(args[0]);
      }
      return originalListen.apply(this, args);
    };

    const provider = makeProvider({ callbackPort: port });
    try {
      const error = await provider.ensureCallbackServerBound().then(
        () => null,
        (e) => e,
      );

      expect(error).not.toBeNull();
      expect(error.code).toBe('CONFIG_ERROR');
      expect(error.message).toContain(`port ${port} is already in use`);
      expect(error.message).toContain('--callback-port');
      expect(provider.callbackServer).toBeUndefined();

      // Only the published port was ever attempted: a listener on any other port would
      // be one the authorization server was never told about.
      expect([...new Set(attemptedPorts)]).toEqual([port]);
    } finally {
      net.Server.prototype.listen = originalListen;
      stopProvider(provider);
      await closeServer(squatter);
    }
  });

  test('the authorization request is not sent when the callback port cannot be bound', async () => {
    // The refusal must come from the bind failure, not from the non-interactive guard.
    delete process.env.PIERRE_DISABLE_BROWSER;
    delete process.env.CI;
    delete process.env.GITHUB_ACTIONS;

    const port = await freePort();
    const squatter = await occupyPort(port);
    const provider = makeProvider({ callbackPort: port });

    try {
      await expect(
        provider.redirectToAuthorization(
          new URL('http://localhost:8081/oauth2/authorize?client_id=test'),
        ),
      ).rejects.toThrow(new RegExp(`port ${port} is already in use`));
    } finally {
      stopProvider(provider);
      await closeServer(squatter);
    }
  });
});
