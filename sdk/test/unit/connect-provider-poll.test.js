// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: connect_provider has Dravr's connect_provider MCP tool mint the page, then polls get_connection_status
// ABOUTME: Drives the real oauth-mode bridge (a delegated grant) and MCP client against a scripted Dravr, end to end

const {
  PROVIDER_PAGE,
  fastPoll,
  jwtFor,
  startDravr,
  wiredBridge,
} = require('../helpers/provider-connect-dravr.js');

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * The status reads `bridge` issues, recorded as it issues them. Whether a read outlives
 * the call is a question of when it was issued, which Dravr's record cannot answer: the
 * wait aborts the read in flight when it ends, and a request sent just before that can
 * reach the server after the call has returned.
 */
function issuedReads(bridge) {
  const issued = [];
  const client = bridge.pierreClient;
  const callTool = client.callTool.bind(client);
  client.callTool = (params, options) => {
    if (params.name === 'get_connection_status') issued.push(params);
    return callTool(params, options);
  };
  return issued;
}

describe('connect_provider mints the page over MCP and polls Dravr for the connection', () => {
  let dravr;
  afterEach(async () => {
    if (dravr) {
      await dravr.close();
      dravr = undefined;
    }
  });

  test('a delegated grant completes end to end: the MCP tool mints the page, no REST launch is asked', async () => {
    dravr = await startDravr(['disconnected', 'disconnected', 'disconnected', 'connected']);
    const wired = await wiredBridge(dravr);
    const waits = fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(wired.logs).toContain(`OAuth URL: ${PROVIDER_PAGE}`);
      // The host's own budget, unchanged by the poll.
      expect(waits).toEqual([{ provider: 'strava', ms: 55000 }]);

      // One read before the page opened, then reads until the fourth said connected.
      const reads = dravr.statusReads();
      expect(reads).toHaveLength(4);
      for (const read of reads) {
        expect(read.rpc.params.arguments).toEqual({ provider: 'strava' });
        expect(read.headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
      }

      // The flow starts on Dravr's connect_provider tool, over the same MCP session and
      // bearer as the reads: the tool takes the athlete from the credential, so no user id
      // is sent. The page opened is the one the tool minted.
      const mints = dravr.mints();
      expect(mints).toHaveLength(1);
      expect(mints[0].rpc.params.arguments).toEqual({ provider: 'strava' });
      expect(mints[0].headers.authorization).toBe(`Bearer ${jwtFor('user-1')}`);
      expect(mints[0].headers['x-callback-token']).toBeUndefined();
      expect(wired.logs).toContain(`Opened strava OAuth in browser: ${PROVIDER_PAGE}`);

      // Every request went to /mcp: no REST launch route, which refuses a delegated
      // grant, was asked. Nothing to call this machine back with was sent, and no
      // listener is bound for it.
      expect(dravr.restCalls()).toEqual([]);
      expect(dravr.seen.map((r) => r.rpc?.params?.name ?? r.rpc?.method)).toEqual([
        'server/discover',
        'get_connection_status',
        'connect_provider',
        'get_connection_status',
        'get_connection_status',
        'get_connection_status',
      ]);
      expect(wired.provider.callbackServer).toBeUndefined();
    } finally {
      await wired.cleanup();
    }
  });

  test('a session token the bridge cannot read still starts the flow: the credential names the athlete', async () => {
    dravr = await startDravr(['disconnected', 'connected']);
    const wired = await wiredBridge(dravr, 'opaque-session-token');
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      const mints = dravr.mints();
      expect(mints).toHaveLength(1);
      expect(mints[0].headers.authorization).toBe('Bearer opaque-session-token');
      expect(dravr.restCalls()).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });

  test('a failed status read is asked again rather than taken for an outcome', async () => {
    dravr = await startDravr(['disconnected', 'error', 'connected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(dravr.statusReads()).toHaveLength(3);
      expect(wired.logs.some((line) => /Connection status read failed, asking again/.test(line))).toBe(true);
    } finally {
      await wired.cleanup();
    }
  });

  test('a connection that needs reauthorizing is authorized again, and only a usable one ends the wait', async () => {
    dravr = await startDravr(['needs_reauth', 'needs_reauth', 'connected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.content[0].text).toMatch(/^Strava connected successfully!/);
      expect(dravr.mints()).toHaveLength(1);
      expect(dravr.statusReads()).toHaveLength(3);
    } finally {
      await wired.cleanup();
    }
  });

  test('times out cleanly: the budget spent is reported as pending and no read outlives the call', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge, 150);
    const issued = issuedReads(wired.bridge);
    try {
      const startedAt = Date.now();
      const result = await wired.connect();
      const elapsed = Date.now() - startedAt;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/STRAVA authorization is still open in your browser and is not confirmed yet/);
      expect(result.content[0].text).toMatch(/run connect_provider again/);
      expect(result.content[0].text).not.toMatch(/connected successfully/i);
      expect(elapsed).toBeLessThan(2000);

      const readsAtReturn = issued.length;
      // The pre-check plus several polls inside a 150ms budget read every 10ms.
      expect(readsAtReturn).toBeGreaterThan(3);
      await pause(100);
      expect(issued).toHaveLength(readsAtReturn);
      // Everything Dravr received was issued before the call returned.
      expect(dravr.statusReads().length).toBeLessThanOrEqual(readsAtReturn);
    } finally {
      await wired.cleanup();
    }
  });

  test('the host cancelling the request ends the poll at once', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    fastPoll(wired.bridge);
    const issued = issuedReads(wired.bridge);
    const controller = new AbortController();
    try {
      const pending = wired.connect({ signal: controller.signal });
      await pause(60);
      controller.abort();
      const result = await pending;

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/Stopped waiting for STRAVA authorization/);

      const readsAtReturn = issued.length;
      expect(readsAtReturn).toBeGreaterThan(1);
      await pause(80);
      expect(issued).toHaveLength(readsAtReturn);
      expect(dravr.statusReads().length).toBeLessThanOrEqual(readsAtReturn);
    } finally {
      await wired.cleanup();
    }
  });

  test('a provider already connected opens no page and polls nothing', async () => {
    dravr = await startDravr(['connected']);
    const wired = await wiredBridge(dravr);
    const waits = fastPoll(wired.bridge);
    try {
      const result = await wired.connect();

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Already connected to STRAVA!/);
      expect(dravr.mints()).toHaveLength(0);
      expect(dravr.restCalls()).toEqual([]);
      expect(dravr.statusReads()).toHaveLength(1);
      expect(waits).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });
});
