// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The two legs of the bridge advertise the same tools — stdio to a host, HTTP to Dravr
// ABOUTME: Both are compared against the generated roster, so a silent disappearance fails here

/**
 * The bridge is dual-era by design: a host speaks to it over stdio at whatever
 * revision it negotiates, and it speaks to Dravr over Streamable HTTP at
 * 2026-07-28. This is the one place both legs are driven at once, so a tool
 * that reaches one and not the other is caught.
 *
 * The expectation on both sides is `TOOL_NAMES`, generated from a live
 * `tools/list`. It replaces a jest snapshot of the same names that CI ran with
 * `--updateSnapshot` — which rewrote the expectation on every run rather than
 * failing when a tool vanished, the exact thing the file was written to catch.
 */

const { McpHttpClient, TOOL_NAMES } = require('../../dist/index.js');
const { ensureServerRunning } = require('../helpers/server');
const { MockMCPClient } = require('../helpers/mock-client');
const { MCPMessages, TestConfig } = require('../helpers/fixtures');
const { clearKeychainTokens } = require('../helpers/keychain-cleanup');
const path = require('path');

describe('Transport parity', () => {
  let serverHandle;
  let testToken;
  const bridgePath = path.join(__dirname, '../../dist/cli.js');
  const serverUrl = `http://localhost:${TestConfig.defaultServerPort}`;
  const mcpUrl = `${serverUrl}/mcp`;

  beforeAll(async () => {
    serverHandle = await ensureServerRunning({
      port: TestConfig.defaultServerPort,
      database: TestConfig.testDatabase,
      encryptionKey: TestConfig.testEncryptionKey,
    });
    testToken = serverHandle?.testToken;
  }, 60000);

  beforeEach(async () => {
    await clearKeychainTokens();
  });

  afterAll(async () => {
    if (serverHandle?.cleanup) {
      await serverHandle.cleanup();
    }
  });

  test('the host leg and the Dravr leg list the same tools, and it is the generated roster', async () => {
    const direct = new McpHttpClient({
      url: mcpUrl,
      clientInfo: { name: 'http-parity', version: '0.0.0' },
      bearer: () => testToken.access_token,
    });

    let httpToolNames;
    try {
      const { tools } = await direct.listTools();
      httpToolNames = tools.map((t) => t.name).sort();
    } finally {
      direct.close();
    }

    // The stdio leg, driven exactly as a host drives it: spawn the built CLI,
    // initialize, then ask for the list.
    const stdioClient = new MockMCPClient('node', [bridgePath, '--server', serverUrl], {
      env: { PIERRE_JWT_TOKEN: testToken.access_token },
    });

    let stdioToolNames;
    try {
      await stdioClient.start();
      await stdioClient.send(MCPMessages.initialize);
      await new Promise((resolve) => setTimeout(resolve, 2000));

      const stdioTools = await stdioClient.send(MCPMessages.toolsList);
      stdioToolNames = stdioTools.result.tools.map((t) => t.name).sort();
    } finally {
      await stdioClient.stop();
    }

    expect(httpToolNames).toEqual(stdioToolNames);
    expect(stdioToolNames).toEqual([...TOOL_NAMES]);
  }, 90000);
});
