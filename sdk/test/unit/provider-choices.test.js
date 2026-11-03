// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The providers the client names come from Dravr's get_connection_status, never from a list of its own
// ABOUTME: Covers the answer after connect_to_dravr and a connect_provider call that names no provider (carnet#1361)

const { OFFERED_PROVIDERS, startDravr, wiredBridge } = require('../helpers/provider-connect-dravr.js');

const OFFERED = Object.keys(OFFERED_PROVIDERS);

describe('the bridge names the providers Dravr offers', () => {
  let dravr;
  afterEach(async () => {
    if (dravr) {
      await dravr.close();
      dravr = undefined;
    }
  });

  test('after connect_to_dravr, every offered provider is listed with its state', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    try {
      const message = await wired.bridge.connectedToDravrMessage('Successfully connected to Dravr!');

      expect(message).toContain(
        '- **garmin** (connected)\n- **strava** (disconnected)\n- **whoop** (needs reauth)',
      );
      expect(dravr.statusReads().map((r) => r.rpc.params.arguments)).toEqual([{}]);
    } finally {
      await wired.cleanup();
    }
  });

  test('connect_to_dravr when already signed in lists the providers too', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    try {
      const result = await wired.bridge.mcpServer._requestHandlers.get('tools/call')(
        { method: 'tools/call', params: { name: 'connect_to_dravr', arguments: {} } },
        { signal: new AbortController().signal },
      );

      expect(result.isError).toBe(false);
      expect(result.content[0].text).toMatch(/^Already connected to Dravr!/);
      expect(result.content[0].text).toContain(
        '- **garmin** (connected)\n- **strava** (disconnected)\n- **whoop** (needs reauth)',
      );
    } finally {
      await wired.cleanup();
    }
  });

  test('when Dravr cannot list them, the answer names no provider rather than inventing one', async () => {
    dravr = await startDravr(['list_error']);
    const wired = await wiredBridge(dravr);
    try {
      const message = await wired.bridge.connectedToDravrMessage('Successfully connected to Dravr!');

      expect(message).toContain('get_connection_status');
      for (const name of [...OFFERED, 'Strava']) {
        expect(message).not.toContain(name);
      }
    } finally {
      await wired.cleanup();
    }
  });

  test('connect_provider without a provider starts no flow and asks which, listing what Dravr offers', async () => {
    dravr = await startDravr(['disconnected']);
    const wired = await wiredBridge(dravr);
    try {
      const result = await wired.bridge.mcpServer._requestHandlers.get('tools/call')(
        { method: 'tools/call', params: { name: 'connect_provider', arguments: {} } },
        { signal: new AbortController().signal },
      );

      expect(result.isError).toBe(true);
      expect(result.content[0].text).toMatch(/^Which provider should I connect\?/);
      for (const name of OFFERED) {
        expect(result.content[0].text).toContain(`**${name}**`);
      }
      expect(dravr.mints()).toEqual([]);
    } finally {
      await wired.cleanup();
    }
  });
});
