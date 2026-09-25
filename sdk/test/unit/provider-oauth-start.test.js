// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The bridge starts a provider's OAuth flow itself and reads WHOOP's notice refusal as a message, not a raw 400
// ABOUTME: Pins the refusal message, the minted authorization URL it opens directly, and the fallback to the initiate route

const http = require('http');
const { startProviderOAuth, providerNoticeMessage, isNoticeRefusal } = require('../../dist/index.js');

/** A one-route server answering the initiate request as the Dravr server would. */
function serve(handler) {
  return new Promise((resolve) => {
    const seen = [];
    const server = http.createServer((req, res) => {
      seen.push({
        url: req.url,
        authorization: req.headers.authorization,
        callbackToken: req.headers['x-callback-token'],
      });
      handler(req, res);
    });
    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address();
      resolve({ server, seen, base: `http://127.0.0.1:${port}` });
    });
  });
}

const INITIATE = '/api/oauth/auth/whoop/user-1';

/** The bridge callback listener's per-flow token, as the SDK mints it. */
const LISTENER_TOKEN = 'c'.repeat(64);

describe('startProviderOAuth', () => {
  let running;
  afterEach(() => new Promise((resolve) => (running ? running.server.close(resolve) : resolve())));

  test('reads the notice refusal as where to accept it, not a raw error', async () => {
    running = await serve((_req, res) => {
      res.writeHead(400, { 'content-type': 'application/json' });
      res.end(
        JSON.stringify({
          code: 'InvalidInput',
          message: 'Connecting WHOOP requires accepting the account notice first',
          details: { action: 'accept_provider_notice', provider: 'whoop' },
        }),
      );
    });

    const start = await startProviderOAuth(`${running.base}${INITIATE}`, 'jwt-1', 'whoop', LISTENER_TOKEN);

    expect(start.kind).toBe('notice_required');
    expect(start.message).toBe(providerNoticeMessage('whoop'));
    expect(start.message).toContain('Open the Dravr app, go to Connections and connect WHOOP');
    expect(start.message).not.toContain('400');
    expect(running.seen).toEqual([
      { url: INITIATE, authorization: 'Bearer jwt-1', callbackToken: LISTENER_TOKEN },
    ]);
  });

  test('opens the authorization page the server minted, so the flow is started once', async () => {
    running = await serve((_req, res) => {
      res.writeHead(302, { location: 'https://api.prod.whoop.com/oauth/oauth2/auth?state=abc' });
      res.end();
    });

    const start = await startProviderOAuth(`${running.base}${INITIATE}`, 'jwt-1', 'whoop', LISTENER_TOKEN);

    expect(start).toEqual({ kind: 'authorize', url: 'https://api.prod.whoop.com/oauth/oauth2/auth?state=abc' });
    expect(running.seen).toHaveLength(1);
    // The flow starts with the listener's token in a header, never in the URL.
    expect(running.seen[0].callbackToken).toBe(LISTENER_TOKEN);
    expect(running.seen[0].url).toBe(INITIATE);
  });

  test('leaves any other answer to the initiate route in the browser', async () => {
    running = await serve((_req, res) => {
      res.writeHead(500, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ code: 'InternalError', message: 'boom' }));
    });

    const start = await startProviderOAuth(`${running.base}${INITIATE}`, 'jwt-1', 'whoop', LISTENER_TOKEN);

    expect(start).toEqual({ kind: 'open_initiate' });
  });

  test('recognizes only the notice refusal', () => {
    expect(isNoticeRefusal(400, { details: { action: 'accept_provider_notice' } })).toBe(true);
    expect(isNoticeRefusal(400, { details: { action: 'connect_provider' } })).toBe(false);
    expect(isNoticeRefusal(403, { details: { action: 'accept_provider_notice' } })).toBe(false);
    expect(isNoticeRefusal(400, null)).toBe(false);
  });
});
